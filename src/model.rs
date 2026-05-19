use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct ApiResponse {
    pub err: i64,
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub result: Option<Value>,
}

pub fn status_of(result: &Value) -> Option<String> {
    result
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// True if the status equals "UNKNOWN" (case-insensitive).
pub fn is_unknown_status(result: &Value) -> bool {
    status_of(result)
        .map(|s| s.eq_ignore_ascii_case("UNKNOWN"))
        .unwrap_or(false)
}

/// True if the status equals "COMPLETED" (case-insensitive).
pub fn is_completed_status(result: &Value) -> bool {
    status_of(result)
        .map(|s| s.eq_ignore_ascii_case("COMPLETED"))
        .unwrap_or(false)
}

pub fn label(key: &str) -> &str {
    match key {
        "order_id" => "订单号",
        "status" => "状态",
        "stop" => "剩余站点",
        "duration" => "预计时长(秒)",
        "length" => "距离(米)",
        "estimated_slot" => "预计时段",
        "completed_time" => "完成时间",
        "addr" => "地址",
        other => other,
    }
}

#[derive(Debug, PartialEq)]
pub struct FieldChange {
    pub key: String,
    pub old: Option<Value>,
    pub new: Option<Value>,
}

/// Diff two `result` objects. Includes modified, added (old=None) and
/// removed (new=None) keys. Stable order (sorted keys).
pub fn diff_snapshots(old: &Value, new: &Value) -> Vec<FieldChange> {
    let empty = serde_json::Map::new();
    let o = old.as_object().unwrap_or(&empty);
    let n = new.as_object().unwrap_or(&empty);
    let mut keys: Vec<String> = o.keys().chain(n.keys()).cloned().collect();
    keys.sort();
    keys.dedup();
    let mut out = Vec::new();
    for k in keys {
        let ov = o.get(&k);
        let nv = n.get(&k);
        if ov != nv {
            out.push(FieldChange {
                key: k,
                old: ov.cloned(),
                new: nv.cloned(),
            });
        }
    }
    out
}

fn val_str(v: &Option<Value>) -> String {
    match v {
        None => "—".to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

fn order_id_of(result: &Value) -> String {
    result
        .get("order_id")
        .and_then(|v| v.as_str())
        .unwrap_or("?")
        .to_string()
}

/// Multi-line current status, used for the join receipt.
pub fn render_status(result: &Value) -> String {
    let mut s = format!("✅ 已加入追踪 {}\n", order_id_of(result));
    for k in ["status", "estimated_slot", "stop", "completed_time", "addr"] {
        if let Some(v) = result.get(k) {
            s.push_str(&format!("{}: {}\n", label(k), val_str(&Some(v.clone()))));
        }
    }
    s.trim_end().to_string()
}

/// One-line summary for `/list`.
pub fn render_summary(result: &Value) -> String {
    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    format!("{} · {}", order_id_of(result), status)
}

pub fn render_changes(order_id: &str, changes: &[FieldChange]) -> String {
    let mut s = format!("🚚 订单 {order_id} 有更新:\n");
    for c in changes {
        s.push_str(&format!(
            "{}: {} → {}\n",
            label(&c.key),
            val_str(&c.old),
            val_str(&c.new)
        ));
    }
    s.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_real_response() {
        let raw =
            r#"{"err":0,"msg":"","result":{"order_id":"000039752","status":"PROCESS","stop":99}}"#;
        let r: ApiResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(r.err, 0);
        let res = r.result.unwrap();
        assert_eq!(status_of(&res).as_deref(), Some("PROCESS"));
        assert!(!is_unknown_status(&res));
    }

    #[test]
    fn unknown_detection_case_insensitive() {
        assert!(is_unknown_status(&json!({"status": "UNKNOWN"})));
        assert!(is_unknown_status(&json!({"status": "unknown"})));
        assert!(!is_unknown_status(&json!({"status": "PROCESS"})));
        assert!(!is_unknown_status(&json!({})));
    }

    #[test]
    fn completed_detection_case_insensitive() {
        assert!(is_completed_status(&json!({"status": "COMPLETED"})));
        assert!(is_completed_status(&json!({"status": "completed"})));
        assert!(!is_completed_status(&json!({"status": "DELIVERED"})));
        assert!(!is_completed_status(&json!({})));
    }

    #[test]
    fn diff_modified_added_removed() {
        let old = json!({"status":"PROCESS","stop":99,"gone":1});
        let new = json!({"status":"DELIVERED","stop":99,"added":"x"});
        let d = diff_snapshots(&old, &new);
        assert_eq!(
            d,
            vec![
                FieldChange {
                    key: "added".into(),
                    old: None,
                    new: Some(json!("x"))
                },
                FieldChange {
                    key: "gone".into(),
                    old: Some(json!(1)),
                    new: None
                },
                FieldChange {
                    key: "status".into(),
                    old: Some(json!("PROCESS")),
                    new: Some(json!("DELIVERED"))
                },
            ]
        );
    }

    #[test]
    fn diff_no_change_is_empty() {
        let v = json!({"status":"PROCESS"});
        assert!(diff_snapshots(&v, &v).is_empty());
    }

    #[test]
    fn render_changes_uses_labels() {
        let changes = vec![FieldChange {
            key: "status".into(),
            old: Some(json!("PROCESS")),
            new: Some(json!("DELIVERED")),
        }];
        let out = render_changes("000039752", &changes);
        assert!(out.contains("订单 000039752 有更新"));
        assert!(out.contains("状态: PROCESS → DELIVERED"));
    }
}
