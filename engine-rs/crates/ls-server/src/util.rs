// 2026-11-12 batch A split (util). No self-glob here.
use crate::state::Shared;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 本地 ISO 秒时间戳（统一走 `ls_core::time` 本地时区，与 Python datetime.now() 同口径）
pub(crate) fn iso_now() -> String {
    ls_core::time::local_now_iso()
}

/// 本地 ISO 日期（days 天前）
pub(crate) fn iso_days_ago(days: i64) -> String {
    ls_core::time::local_days_ago_date(days)
}

pub(crate) fn rooted_file(
    base: &std::path::Path,
    path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let base = base.canonicalize().ok()?;
    let path = base.join(path).canonicalize().ok()?;
    (path.starts_with(base) && path.is_file()).then_some(path)
}

/// secrets.token_urlsafe(32) 等价：32 系统熵字节 → base64url（无 padding）
pub(crate) fn random_token() -> String {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("system entropy");
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    // 6-bit 打包：43 字符承载 258 bit >= 256 bit
    let mut out = String::with_capacity(43);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &b in buf.iter() {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((acc >> bits) & 0x3f) as usize] as char);
        }
    }
    out
}

pub(crate) fn uuid_like() -> String {
    // 32 hex（与 Python uuid4().hex 同形，无需外部 crate）
    // P2-9（安全审计）：原为纳秒时间戳——可预测且同纳秒碰撞，不满足审计
    // 关联唯一性。改为 getrandom 熵（已在 deps）+ 时间混合。
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed) as u128;
    let mut seed = (t ^ (seq << 64)).to_le_bytes();
    // getrandom 随机熵填充（getrandom 0.2 API：getrandom(&mut buf)）
    let _ = getrandom::getrandom(&mut seed);
    format!("{:032x}", u128::from_le_bytes(seed))
}

// ---- handlers ----

pub(crate) fn with_store<F, R>(st: &Shared, f: F) -> Response
where
    F: FnOnce(&ls_store::Store) -> rusqlite::Result<R>,
    R: IntoResponse,
{
    match ls_store::Store::open(&st.db_path) {
        Ok(s) => match f(&s) {
            Ok(v) => v.into_response(),
            Err(_) => server_err(),
        },
        Err(_) => server_err(),
    }
}

pub(crate) fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 生成本地分钟时间戳 `YYYY-MM-DD HH:MM`。
pub(crate) fn now_minute() -> String {
    let dt = ls_core::time::local_now_civil();
    format!(
        "{} {}",
        ls_core::time::format_date(dt),
        ls_core::time::format_hm(dt)
    )
}

/// Python `_RTSP_CRED` 同款：rtsp://user:pass@ → rtsp://user:***:***@
pub(crate) fn mask_rtsp_creds(source: &str) -> String {
    // 简单可靠的做法：找到 "://" 后第一个 @，把其中的 "pass:" 部分掩掉
    let Some(idx) = source.find("://") else {
        return source.to_string();
    };
    let (scheme, rest) = source.split_at(idx + 3);
    match rest.find('@') {
        Some(at) if !rest[..at].is_empty() => {
            let cred = &rest[..at];
            let masked = match cred.split_once(':') {
                Some((user, _)) => format!("{user}:***:***@"),
                None => "***:***@".to_string(),
            };
            format!("{scheme}{masked}{}", &rest[at + 1..])
        }
        _ => source.to_string(),
    }
}

pub(crate) fn file_info(p: &std::path::Path) -> Value {
    match std::fs::metadata(p) {
        Ok(m) => json!({"path": p.to_string_lossy(), "exists": true,
                        "size_bytes": m.len(),
                        "mtime": iso_now()}),
        Err(_) => json!({"path": p.to_string_lossy(), "exists": false,
                         "size_bytes": null, "mtime": null}),
    }
}

/// 统一 4xx/5xx JSON 错误体（`{"detail": msg}`）——2026-11-12 深审：手写
/// `detail_err(StatusCode::X, "...").into_response()` 样板在
/// 全文件出现约 70 次（纯字面量形态），收敛到此 helper（带 code 的复合错误体
/// 与动态 msg 保留原写法）。
pub(crate) fn detail_err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"detail": msg}))).into_response()
}

pub(crate) fn server_err() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"detail": "服务器内部错误", "code": "internal_error"})),
    )
        .into_response()
}
