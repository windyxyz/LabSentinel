//! 8 维状态卡尔曼滤波（xyah）：与 supervision/tracker/byte_tracker/kalman_filter.py
//! 逐数值对齐（T-324）。f32 运算与 numpy float32 的舍入一致（向量容差 1e-3）。
// 矩阵维度固定为 8x8/4x4，手写索引循环比迭代器组合更贴近论文公式、便于逐元素对齐审计
#![allow(clippy::needless_range_loop)]

pub const STATE_DIM: usize = 8; // x, y, a, h, vx, vy, va, vh

const STD_WEIGHT_POSITION: f32 = 1.0 / 20.0;
const STD_WEIGHT_VELOCITY: f32 = 1.0 / 160.0;

/// 8x8 对称矩阵（行主序展平，无外部线性代数依赖——维度固定，手写足够快）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat8(pub [[f32; STATE_DIM]; STATE_DIM]);

impl Mat8 {
    pub fn diag(d: [f32; STATE_DIM]) -> Self {
        let mut m = [[0.0f32; STATE_DIM]; STATE_DIM];
        for i in 0..STATE_DIM {
            m[i][i] = d[i];
        }
        Mat8(m)
    }

    pub fn identity() -> Self {
        let mut m = [[0.0f32; STATE_DIM]; STATE_DIM];
        for i in 0..STATE_DIM {
            m[i][i] = 1.0;
        }
        Mat8(m)
    }
}

/// 运动矩阵 F（4 维位置 + 4 维速度，dt=1）与观测矩阵 H（取前 4 维）。
fn motion_mat() -> Mat8 {
    let mut m = Mat8::identity().0;
    for i in 0..4 {
        m[i][4 + i] = 1.0; // dt = 1.0
    }
    Mat8(m)
}

/// mean' = F·mean；cov' = F·cov·Fᵀ + Q(h)
pub fn predict(mean: [f32; STATE_DIM], cov: &Mat8) -> ([f32; STATE_DIM], Mat8) {
    let f = motion_mat();
    let h = mean[3];
    let q = Mat8::diag([
        (STD_WEIGHT_POSITION * h).powi(2),
        (STD_WEIGHT_POSITION * h).powi(2),
        1e-2f32.powi(2),
        (STD_WEIGHT_POSITION * h).powi(2),
        (10.0 * STD_WEIGHT_VELOCITY * h).powi(2),
        (10.0 * STD_WEIGHT_VELOCITY * h).powi(2),
        1e-5f32.powi(2),
        (10.0 * STD_WEIGHT_VELOCITY * h).powi(2),
    ]);
    let mut new_mean = [0.0f32; STATE_DIM];
    for i in 0..STATE_DIM {
        let mut acc = 0.0;
        for j in 0..STATE_DIM {
            acc += f.0[i][j] * mean[j];
        }
        new_mean[i] = acc;
    }
    let new_cov = mat_mul(mat_mul(f.0, cov.0), transpose(&f.0));
    (new_mean, Mat8(add(&new_cov, &q.0)))
}

/// 观测投影：H·mean、H·cov·Hᵀ + R(h)（4x4）
pub fn project(mean: [f32; STATE_DIM], cov: &Mat8) -> ([f32; 4], [[f32; 4]; 4]) {
    let h = mean[3];
    let r = [
        (STD_WEIGHT_POSITION * h).powi(2),
        (STD_WEIGHT_POSITION * h).powi(2),
        1e-1f32.powi(2),
        (STD_WEIGHT_POSITION * h).powi(2),
    ];
    let mut proj_mean = [0.0f32; 4];
    proj_mean.copy_from_slice(&mean[..4]); // H = [I4 | 0]
                                           // H·cov·Hᵀ = cov 的左上 4x4 子阵
    let mut proj_cov = [[0.0f32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            proj_cov[i][j] = cov.0[i][j] + (if i == j { r[i] } else { 0.0 });
        }
    }
    (proj_mean, proj_cov)
}

/// 更新步：Kalman 增益经 4x4 Cholesky 求解（与 scipy cho_factor/cho_solve 等价路径）。
pub fn update(
    mean: [f32; STATE_DIM],
    cov: &Mat8,
    measurement: [f32; 4],
) -> ([f32; STATE_DIM], Mat8) {
    let (proj_mean, proj_cov) = project(mean, cov);
    // K = (cov·Hᵀ)·S⁻¹，其中 S=proj_cov（4x4）；解 S·X = (cov·Hᵀ)ᵀ 再转置
    let ph = covariance_times_h(cov); // 8x4 = cov·Hᵀ
    let ph_t = transpose4(&ph); // 4x8
    let x = solve_spd(&proj_cov, &ph_t); // X = S⁻¹·(PHᵀ)ᵀ
    let k = transpose4_8(&x); // K = Xᵀ (8x4)

    let mut innovation = [0.0f32; 4];
    for i in 0..4 {
        innovation[i] = measurement[i] - proj_mean[i];
    }
    let mut new_mean = mean;
    for i in 0..STATE_DIM {
        let mut acc = 0.0;
        for j in 0..4 {
            acc += k[i][j] * innovation[j];
        }
        new_mean[i] += acc;
    }
    // new_cov = cov − K·S·Kᵀ（等价 cov − K·PHᵀ，数值更稳的展开）
    let mut kph = [[0.0f32; STATE_DIM]; STATE_DIM];
    for i in 0..STATE_DIM {
        for j in 0..STATE_DIM {
            let mut acc = 0.0;
            for l in 0..4 {
                acc += k[i][l] * ph[j][l];
            }
            kph[i][j] = acc;
        }
    }
    let mut new_cov = [[0.0f32; STATE_DIM]; STATE_DIM];
    for i in 0..STATE_DIM {
        for j in 0..STATE_DIM {
            new_cov[i][j] = cov.0[i][j] - kph[i][j];
        }
    }
    (new_mean, Mat8(new_cov))
}

fn covariance_times_h(cov: &Mat8) -> [[f32; 4]; STATE_DIM] {
    let mut out = [[0.0f32; 4]; STATE_DIM];
    for i in 0..STATE_DIM {
        for j in 0..4 {
            out[i][j] = cov.0[i][j]; // Hᵀ = [I4; 0]
        }
    }
    out
}

fn add(
    a: &[[f32; STATE_DIM]; STATE_DIM],
    b: &[[f32; STATE_DIM]; STATE_DIM],
) -> [[f32; STATE_DIM]; STATE_DIM] {
    let mut m = [[0.0f32; STATE_DIM]; STATE_DIM];
    for i in 0..STATE_DIM {
        for j in 0..STATE_DIM {
            m[i][j] = a[i][j] + b[i][j];
        }
    }
    m
}

fn transpose(a: &[[f32; STATE_DIM]; STATE_DIM]) -> [[f32; STATE_DIM]; STATE_DIM] {
    let mut m = [[0.0f32; STATE_DIM]; STATE_DIM];
    for i in 0..STATE_DIM {
        for j in 0..STATE_DIM {
            m[i][j] = a[j][i];
        }
    }
    m
}

fn mat_mul(
    a: [[f32; STATE_DIM]; STATE_DIM],
    b: [[f32; STATE_DIM]; STATE_DIM],
) -> [[f32; STATE_DIM]; STATE_DIM] {
    let mut m = [[0.0f32; STATE_DIM]; STATE_DIM];
    for i in 0..STATE_DIM {
        for j in 0..STATE_DIM {
            let mut acc = 0.0;
            for l in 0..STATE_DIM {
                acc += a[i][l] * b[l][j];
            }
            m[i][j] = acc;
        }
    }
    m
}

fn transpose4(a: &[[f32; 4]; STATE_DIM]) -> [[f32; STATE_DIM]; 4] {
    let mut m = [[0.0f32; STATE_DIM]; 4];
    for i in 0..4 {
        for j in 0..STATE_DIM {
            m[i][j] = a[j][i];
        }
    }
    m
}

/// (4x8)ᵀ → 8x4（update 步专用）
fn transpose4_8(a: &[[f32; STATE_DIM]; 4]) -> [[f32; 4]; STATE_DIM] {
    let mut m = [[0.0f32; 4]; STATE_DIM];
    for i in 0..4 {
        for j in 0..STATE_DIM {
            m[j][i] = a[i][j];
        }
    }
    m
}

/// 解 SPD 方程 S·X = B（S: 4x4 对称正定；Cholesky，对齐 scipy.linalg.cho_factor/cho_solve）。
fn solve_spd(s: &[[f32; 4]; 4], b: &[[f32; STATE_DIM]; 4]) -> [[f32; STATE_DIM]; 4] {
    let n = 4usize;
    let mut l = [[0.0f32; 4]; 4];
    for i in 0..n {
        for j in 0..=i {
            let mut acc = s[i][j];
            for k in 0..j {
                acc -= l[i][k] * l[j][k];
            }
            if i == j {
                l[i][j] = acc.max(1e-12).sqrt();
            } else {
                l[i][j] = acc / l[j][j];
            }
        }
    }
    // 前代 L·Y = B
    let mut y = [[0.0f32; STATE_DIM]; 4];
    for i in 0..n {
        for col in 0..STATE_DIM {
            let mut acc = b[i][col];
            for k in 0..i {
                acc -= l[i][k] * y[k][col];
            }
            y[i][col] = acc / l[i][i];
        }
    }
    // 回代 Lᵀ·X = Y
    let mut x = [[0.0f32; STATE_DIM]; 4];
    for i in (0..n).rev() {
        for col in 0..STATE_DIM {
            let mut acc = y[i][col];
            for k in (i + 1)..n {
                acc -= l[k][i] * x[k][col];
            }
            x[i][col] = acc / l[i][i];
        }
    }
    x
}
