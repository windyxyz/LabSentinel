use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use ls_store::Store;
use serde_json::{json, Value};
use sha2::Sha256;

type ReceiptMac = Hmac<Sha256>;

fn mac(key: &str, claims: &Value) -> ReceiptMac {
    let mut signer =
        ReceiptMac::new_from_slice(key.as_bytes()).expect("HMAC accepts all key sizes");
    signer.update(b"LabSentinel MCP receipt v1\0");
    signer.update(claims.to_string().as_bytes());
    signer
}

fn claims_for_event(
    store: &Store,
    id: i64,
    cameras: Option<&[i64]>,
) -> rusqlite::Result<Option<Value>> {
    let Some(event) = store
        .event_by_id(id)?
        .filter(|event| cameras.is_none_or(|scope| scope.contains(&event.camera_id)))
    else {
        return Ok(None);
    };
    Ok(Some(
        json!({"event_id":event.id,"event_ts":event.ts,"snapshot_sha256":event.snapshot_sha256,
        "provenance_hash":store.event_provenance(event.id)?.map(|(hash,_)| hash)}),
    ))
}

pub(super) fn attach(
    store: &Store,
    audit_id: i64,
    tool: &str,
    cameras: Option<&[i64]>,
    outcome: &mut super::tools::Outcome,
) -> rusqlite::Result<()> {
    if !outcome.value.is_object() {
        outcome.value = json!({"data":std::mem::take(&mut outcome.value)});
    }
    let mut event_ids = Vec::new();
    event_ids.extend(outcome.event_id);
    if tool == "list_events" || tool == "semantic_search_events" {
        let (field, id_field) = if tool == "list_events" {
            ("events", "id")
        } else {
            ("items", "event_id")
        };
        if let Some(items) = outcome.value[field].as_array() {
            event_ids.extend(
                items
                    .iter()
                    .take(100)
                    .filter_map(|item| item[id_field].as_i64()),
            );
        }
    }
    event_ids.sort_unstable();
    event_ids.dedup();
    let events = event_ids
        .into_iter()
        .map(|id| claims_for_event(store, id, cameras))
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let result_hash = crate::util::sha256_hex(outcome.value.to_string().as_bytes());
    let claims = json!({"version":1,"audit_id":audit_id,"tool":tool,
        "issued_at":super::now(),"result_sha256":result_hash,"events":events});
    let key = store.mcp_receipt_secret(&crate::util::random_token())?;
    let signature = URL_SAFE_NO_PAD.encode(mac(&key, &claims).finalize().into_bytes());
    outcome.value["_receipt"] = json!({"claims":claims,"signature":signature});
    Ok(())
}

pub(super) fn verify(store: &Store, receipt: &Value) -> rusqlite::Result<bool> {
    let claims = &receipt["claims"];
    let Some(signature) = receipt["signature"]
        .as_str()
        .filter(|s| s.len() == 43)
        .and_then(|s| URL_SAFE_NO_PAD.decode(s).ok())
    else {
        return Ok(false);
    };
    if claims["version"] != 1 {
        return Ok(false);
    }
    Ok(store
        .setting_get("mcp.receipt_secret")?
        .is_some_and(|key| mac(&key, claims).verify_slice(&signature).is_ok()))
}
