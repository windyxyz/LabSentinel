//! ls-core：LabSentinel 领域逻辑（共享测试向量、无 unsafe）。
//!
//! 与 Python 实现（server/app）共享 `testdata/vectors/*.json` 测试向量，
//! 保证 Rust 与 Python 的领域行为逐用例一致（T-323，L0 对齐基准）。
//!
//! 模块：
//! - [`zones`]：区域多边形判定（与 `cv2.pointPolygonTest >= 0` 对齐，边界算在内）
//! - [`state`]：ZoneState 进出迟滞（与 Python `app.zones.ZoneState` 语义一致）
//! - [`bin`]：R2-301 外部二进制（ffmpeg/ffprobe）统一查找（唯一带 IO 的模块）

pub mod bin;
pub mod state;
pub mod zones;

pub use zones::point_in_polygon;

/// 整数点（检测锚点/多边形顶点，与 Python 端 int 坐标一致）。
pub type Point = (i64, i64);

/// 事件变化的统一编码（与 Python `ZoneState.update` 返回的 (entered, exited) 对齐）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneChange {
    /// 进入区域。
    Enter,
    /// 离开区域。
    Exit,
}

impl ZoneChange {
    /// 与共享测试向量的字符串编码一致（"enter" / "exit"）。
    pub fn as_str(self) -> &'static str {
        match self {
            ZoneChange::Enter => "enter",
            ZoneChange::Exit => "exit",
        }
    }
}
