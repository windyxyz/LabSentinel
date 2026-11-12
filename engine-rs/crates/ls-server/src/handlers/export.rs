// CSV export + monthly PDF (pdf:: delegation).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use crate::{discovery, fernet, pdf};
use axum::extract::{
    ws::{WebSocket, WebSocketUpgrade},
    Path as AxPath, Query, State,
};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{delete, get, post, put};
use axum::Router;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn export_events_csv(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.events_query(&ls_store::EventsFilter {
            limit: 10_000,
            offset: 0,
            ..Default::default()
        })?;
        let mut out = String::from("id,ts,camera_id,type,level,track_id,status,detail\n");
        for e in &rows {
            out.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                e.id,
                csv_escape(&e.ts),
                e.camera_id,
                csv_escape(&e.type_),
                e.level,
                e.track_id,
                csv_escape(&e.status),
                csv_escape(&e.detail)
            ));
        }
        Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], out).into_response())
    })
}

pub(crate) async fn export_stats_csv(State(st): State<Shared>) -> Response {
    with_store(&st, |s| {
        let rows = s.stats_daily(30, None, None)?;
        let mut out = String::from("date,total,l1,l2,l3\n");
        for (d, row) in rows {
            let g = |k: &str| row.get(k).copied().unwrap_or(0);
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                d,
                g("total"),
                g("l1"),
                g("l2"),
                g("l3")
            ));
        }
        Ok(([(header::CONTENT_TYPE, "text/csv; charset=utf-8")], out).into_response())
    })
}

/// `YYYY-MM` → (year, month)。对齐 Python `strptime(month, "%Y-%m")` 的常见口径
/// （月允许 1-2 位；年要求 4 位——Python `%Y` 更宽松，此处收紧到实际契约）。
pub(crate) fn parse_month(s: &str) -> Option<(i64, u32)> {
    let (y, m) = s.split_once('-')?;
    if y.len() != 4 || !(1..=2).contains(&m.len()) {
        return None;
    }
    let y: i64 = y.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if !(1..=12).contains(&m) {
        return None;
    }
    Some((y, m))
}

/// 月首/次日边界字符串与当月天数（平闰年规则就地实现，不引新依赖）。
pub(crate) fn month_bounds(y: i64, m: u32) -> (String, String, u32) {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let leap = |yy: i64| (yy % 4 == 0 && yy % 100 != 0) || yy % 400 == 0;
    let dim = [
        31u32,
        if leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ][(m - 1) as usize];
    (
        format!("{y:04}-{m:02}-01"),
        format!("{ny:04}-{nm:02}-01"),
        dim,
    )
}

/// R2-309 余量收口：月报 PDF（版式与 Python `export.monthly_pdf` 对齐，实现见 pdf.rs）。
/// 注意：`events_query` 的 end_ts 是闭区间（`<=`），Python `events_between` 是开区间
/// （`<`）——end 过滤在 handler 内做，保持两侧同口径（单月取 100 万行内全量统计）。
pub(crate) async fn export_monthly_pdf(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let bad = |msg: &str| (StatusCode::BAD_REQUEST, Json(json!({"detail": msg}))).into_response();
    let Some(month) = q.get("month") else {
        // Python FastAPI 缺必填 query 参数返回 422（错误体为 error 对象数组）
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!([{"loc": ["query", "month"], "msg": "Field required", "type": "missing"}])),
        )
            .into_response();
    };
    let Some((y, m)) = parse_month(month) else {
        return bad("month 必须是 YYYY-MM 格式");
    };
    let month = month.clone();
    with_store(&st, |s| {
        let (start, until, dim) = month_bounds(y, m);
        let rows = s.events_query(&ls_store::EventsFilter {
            start_ts: Some(&start),
            limit: 1_000_000,
            ..Default::default()
        })?;
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|e| e.ts.as_str() < until.as_str()) // Python：ts < until（开区间）
            .collect();
        let mut by_level = [0i64; 3];
        let mut unhandled = 0i64;
        let mut by_day: Vec<(u32, i64)> = Vec::new();
        for e in &rows {
            if (1..=3).contains(&e.level) {
                by_level[(e.level - 1) as usize] += 1;
            }
            if e.status == "new" {
                unhandled += 1;
            }
            if let Ok(d) = e.ts.get(8..10).unwrap_or("").parse::<u32>() {
                match by_day.iter_mut().find(|(dd, _)| *dd == d) {
                    Some((_, v)) => *v += 1,
                    None => by_day.push((d, 1)),
                }
            }
        }
        let generated = now_minute();
        let report = pdf::MonthlyReport {
            month: &month,
            generated: &generated,
            total: rows.len() as i64,
            by_level,
            unhandled,
            by_day: &by_day,
            days_in_month: dim,
        };
        let bytes = pdf::render_monthly(&report);
        let ctype = HeaderValue::from_static("application/pdf");
        let disposition =
            HeaderValue::from_str(&format!("attachment; filename=monthly_{month}.pdf"))
                .unwrap_or(HeaderValue::from_static("attachment"));
        Ok((
            [
                (header::CONTENT_TYPE, ctype),
                (header::CONTENT_DISPOSITION, disposition),
            ],
            bytes,
        )
            .into_response())
    })
}

