// stats endpoints (summary/daily/weekly/hourly/quality).
// 2026-11-12 batch A split. All fns pub(crate) for router registration.
// 2026-11-12 batch A split. Unused items in this superset `use` are cleaned by cargo fix.
use crate::state::Shared;
use crate::util::*;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

pub(crate) async fn stats_summary(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .clamp(1, 3650);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    let s = match ls_store::Store::open(&st.db_path) {
        Ok(s) => s,
        Err(_) => {
            return Json(json!({
                "total": 0,
                "by_level": {"1": 0, "2": 0, "3": 0},
                "days": days,
                "top_days": [],
            }))
        }
    };
    let cutoff = ls_core::time::local_days_ago_iso(days);
    let rows: Vec<_> = s
        .events(100000, 0, cid, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.ts >= cutoff)
        .collect();
    let total = rows.len();
    let by_level = json!({"1": rows.iter().filter(|e| e.level == 1).count(),
                          "2": rows.iter().filter(|e| e.level == 2).count(),
                          "3": rows.iter().filter(|e| e.level == 3).count()});
    let mut by_day: std::collections::BTreeMap<String, i64> = Default::default();
    for e in &rows {
        if let Some(d) = e.ts.split('T').next() {
            *by_day.entry(d.to_string()).or_insert(0) += 1;
        }
    }
    let mut top_days: Vec<Value> = by_day
        .into_iter()
        .map(|(date, n)| json!({"date": date, "total": n}))
        .collect();
    top_days.sort_by(|a, b| {
        b["total"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["total"].as_i64().unwrap_or(0))
    });
    top_days.truncate(5);
    Json(json!({"total": total, "by_level": by_level, "days": days, "top_days": top_days}))
}

pub(crate) fn agg_json(row: &ls_store::AggRow) -> Value {
    let mut m = serde_json::Map::new();
    for (k, v) in row {
        m.insert(k.clone(), json!(v));
    }
    Value::Object(m)
}

pub(crate) async fn stats_daily(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let days: i64 = q.get("days").and_then(|v| v.parse().ok()).unwrap_or(7);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    let status = q.get("status").map(|s| s.as_str());
    with_store(&st, |s| {
        let rows = s.stats_daily(days, cid, status)?;
        Ok(Json(json!({"days": rows.into_iter()
            .map(|(d, row)| { let mut v = json!({"date": d}); if let Value::Object(m) = agg_json(&row) { for (k, val) in m { v[k] = val; } } v })
            .collect::<Vec<_>>()})))
    })
}

pub(crate) async fn stats_weekly(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let weeks: i64 = q.get("weeks").and_then(|v| v.parse().ok()).unwrap_or(4);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    let status = q.get("status").map(|s| s.as_str());
    with_store(&st, |s| {
        let rows = s.stats_weekly(weeks, cid, status)?;
        Ok(Json(json!({"weeks": rows.into_iter()
            .map(|(wk, row)| { let mut v = json!({"week_start": wk}); if let Value::Object(m) = agg_json(&row) { for (k, val) in m { v[k] = val; } } v })
            .collect::<Vec<_>>()})))
    })
}

pub(crate) async fn stats_hourly(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let date = q.get("date").cloned().unwrap_or_default();
    if date.len() != 10 || date.is_empty() {
        return detail_err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "date 必须是 YYYY-MM-DD 格式",
        )
        .into_response();
    }
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let rows = s.stats_hourly(&date, cid)?;
        // LS-042 契约修复：返回 24 项有序数组 [{hour: 0..23, total, l1, l2, l3}]
        // （此前返回 JSON Object 导致前端 ReportsView.vue `hourly.value.map(...)` 渲染崩溃闪黑）
        let mut by_h: std::collections::HashMap<u32, Value> = Default::default();
        for (h, row) in rows {
            if let Ok(hn) = h.parse::<u32>() {
                let mut v = json!({"hour": hn, "total": 0, "l1": 0, "l2": 0, "l3": 0});
                if let Value::Object(m) = agg_json(&row) {
                    for (k, val) in m {
                        v[k] = val;
                    }
                }
                by_h.insert(hn, v);
            }
        }
        let hours: Vec<Value> = (0u32..24)
            .map(|h| {
                by_h.remove(&h)
                    .unwrap_or_else(|| json!({"hour": h, "total": 0, "l1": 0, "l2": 0, "l3": 0}))
            })
            .collect();
        Ok(Json(json!({"date": date, "hours": hours})))
    })
}

pub(crate) async fn stats_quality(
    State(st): State<Shared>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(7)
        .clamp(1, 90);
    let cid: Option<i64> = q.get("camera_id").and_then(|v| v.parse().ok());
    with_store(&st, |s| {
        let start = iso_days_ago(days - 1) + "T00:00:00";
        let end = iso_now();
        let mut where_sql = String::from(" AND ts>=? AND ts<=?");
        let mut args: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(start.clone()), Box::new(end.clone())];
        if let Some(c) = cid {
            where_sql.push_str(" AND camera_id=?");
            args.push(Box::new(c));
        }
        let vals: Vec<&dyn rusqlite::ToSql> = args.iter().map(|a| a.as_ref()).collect();
        let total: i64 = s.conn().query_row(
            &format!("SELECT COUNT(*) FROM events WHERE 1=1{where_sql}"),
            vals.as_slice(),
            |r| r.get(0),
        )?;
        let mut by_status = serde_json::Map::new();
        {
            let mut stmt = s.conn().prepare(&format!(
                "SELECT COALESCE(status,'new') k, COUNT(*) n FROM events WHERE 1=1{where_sql} GROUP BY k"))?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?;
            for (k, n) in rows.collect::<Result<Vec<_>, _>>()? {
                by_status.insert(k, json!(n));
            }
        }
        let mut by_level = serde_json::Map::new();
        {
            let mut stmt = s.conn().prepare(&format!(
                "SELECT level k, COUNT(*) n FROM events WHERE 1=1{where_sql} GROUP BY k"
            ))?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })?;
            for (k, n) in rows.collect::<Result<Vec<_>, _>>()? {
                by_level.insert(k.to_string(), json!(n));
            }
        }
        let mut by_type = serde_json::Map::new();
        {
            let mut stmt = s.conn().prepare(&format!(
                "SELECT type k, COUNT(*) n FROM events WHERE 1=1{where_sql} GROUP BY k"
            ))?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?;
            for (k, n) in rows.collect::<Result<Vec<_>, _>>()? {
                by_type.insert(k, json!(n));
            }
        }
        let mut by_camera: Vec<Value> = Vec::new();
        {
            let sql = format!(
                "SELECT e.camera_id, COALESCE(c.name,'') name, COUNT(*) total, \
                 SUM(CASE WHEN e.status='false_positive' THEN 1 ELSE 0 END) fp \
                 FROM events e LEFT JOIN cameras c ON c.id=e.camera_id WHERE 1=1{where_sql} \
                 GROUP BY e.camera_id ORDER BY total DESC"
            );
            let mut stmt = s.conn().prepare(&sql)?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?;
            for (cid2, name, total2, fp) in rows.collect::<Result<Vec<_>, _>>()? {
                by_camera.push(json!({"camera_id": cid2, "name": name, "total": total2,
                    "false_positive": fp,
                    "rate": if total2 > 0 { (fp as f64 / total2 as f64 * 1000.0).round() / 1000.0 } else { 0.0 }}));
            }
        }
        let mut daily: Vec<Value> = Vec::new();
        {
            let sql = format!(
                "SELECT substr(ts,1,10) d, COUNT(*) total, \
                 SUM(CASE WHEN status='false_positive' THEN 1 ELSE 0 END) fp \
                 FROM events WHERE 1=1{where_sql} GROUP BY d ORDER BY d"
            );
            let mut stmt = s.conn().prepare(&sql)?;
            let rows = stmt.query_map(vals.as_slice(), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?;
            for (d, total2, fp) in rows.collect::<Result<Vec<_>, _>>()? {
                daily.push(json!({"date": d, "total": total2, "false_positive": fp}));
            }
        }
        let stat = |m: &serde_json::Map<String, Value>, k: &str| {
            m.get(k).and_then(|v| v.as_i64()).unwrap_or(0)
        };
        let fp = stat(&by_status, "false_positive");
        let handled = stat(&by_status, "acked") + fp + stat(&by_status, "ignored");
        let l3_total = by_level.get("3").and_then(|v| v.as_i64()).unwrap_or(0);
        let unhandled = stat(&by_status, "new");
        Ok(Json(json!({
            "days": days, "start_ts": start, "end_ts": end,
            "total": total,
            "by_status": Value::Object(by_status),
            "by_level": Value::Object(by_level),
            "by_type": Value::Object(by_type),
            "by_camera": by_camera,
            "false_positive_rate": if handled > 0 { (fp as f64 / handled as f64 * 1000.0).round() / 1000.0 } else { 0.0 },
            "unhandled": unhandled,
            "l3_total": l3_total,
            "by_ai_verdict": {"likely_true": 0, "likely_false": 0, "uncertain": 0},
            "daily": daily,
        }))
        .into_response())
    })
}

// ---- 配对/设备（T-620/R2-309） ----
