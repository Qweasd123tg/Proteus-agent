use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::ProcessModuleRpcError;

#[derive(Debug)]
/// A decoded component-v3 envelope, before routing and invocation validation.
pub enum ComponentFrame {
    Response {
        id: String,
        result: Result<Value, ProcessModuleRpcError>,
    },
    Request {
        id: String,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
}

/// Checks the strict JSON-RPC envelope and preserves its opaque payload.
///
/// Wire ID grammar, direction, generation, method authority and invocation
/// state are validated separately by the receiving host or worker.
pub fn parse_component_frame(value: Value) -> Result<ComponentFrame> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC frame must be an object"))?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        bail!("JSON-RPC frame must declare jsonrpc=\"2.0\"");
    }

    match (
        object.contains_key("id"),
        object.contains_key("method"),
        object.contains_key("result"),
        object.contains_key("error"),
    ) {
        (true, false, true, false) => parse_success(object),
        (true, false, false, true) => parse_error(object),
        (true, true, false, false) => parse_request(object),
        (false, true, false, false) => parse_notification(object),
        _ => bail!("invalid or ambiguous JSON-RPC envelope"),
    }
}

fn parse_success(object: &Map<String, Value>) -> Result<ComponentFrame> {
    require_exact_fields(object, &["jsonrpc", "id", "result"])?;
    Ok(ComponentFrame::Response {
        id: string_id(object.get("id").expect("checked id"))?,
        result: Ok(object.get("result").expect("checked result").clone()),
    })
}

fn parse_error(object: &Map<String, Value>) -> Result<ComponentFrame> {
    require_exact_fields(object, &["jsonrpc", "id", "error"])?;
    let error = serde_json::from_value::<ProcessModuleRpcError>(
        object.get("error").expect("checked error").clone(),
    )
    .context("invalid JSON-RPC error body")?;
    Ok(ComponentFrame::Response {
        id: string_id(object.get("id").expect("checked id"))?,
        result: Err(error),
    })
}

fn parse_request(object: &Map<String, Value>) -> Result<ComponentFrame> {
    require_exact_fields(object, &["jsonrpc", "id", "method", "params"])?;
    Ok(ComponentFrame::Request {
        id: string_id(object.get("id").expect("checked id"))?,
        method: method(object)?,
        params: object.get("params").expect("checked params").clone(),
    })
}

fn parse_notification(object: &Map<String, Value>) -> Result<ComponentFrame> {
    require_exact_fields(object, &["jsonrpc", "method", "params"])?;
    Ok(ComponentFrame::Notification {
        method: method(object)?,
        params: object.get("params").expect("checked params").clone(),
    })
}

fn method(object: &Map<String, Value>) -> Result<String> {
    object
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| !method.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC method must be a non-empty string"))
}

fn string_id(value: &Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("component-v3 JSON-RPC id must be a string"))
}

fn require_exact_fields(object: &Map<String, Value>, expected: &[&str]) -> Result<()> {
    if object.len() != expected.len() {
        bail!("JSON-RPC envelope contains unknown or missing fields");
    }
    for field in expected {
        if !object.contains_key(*field) {
            bail!("JSON-RPC envelope is missing field {field:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn envelopes_preserve_requests_notifications_and_terminal_payloads() {
        let params = json!({"export": {"slot": "tool", "module_id": "rg_search"}});
        let ComponentFrame::Request {
            id,
            method,
            params: actual,
        } = parse_component_frame(
            json!({"jsonrpc":"2.0", "id":"h:1:1", "method":"search", "params":params}),
        )
        .expect("valid request")
        else {
            panic!("expected request")
        };
        assert_eq!(id, "h:1:1");
        assert_eq!(method, "search");
        assert_eq!(actual, params);

        let ComponentFrame::Notification { method, params } =
            parse_component_frame(json!({"jsonrpc":"2.0", "method":"cancel", "params":null}))
                .expect("valid notification")
        else {
            panic!("expected notification")
        };
        assert_eq!(method, "cancel");
        assert_eq!(params, Value::Null);

        let ComponentFrame::Response { id, result } =
            parse_component_frame(json!({"jsonrpc":"2.0", "id":"h:1:1", "result":null}))
                .expect("valid success")
        else {
            panic!("expected response")
        };
        assert_eq!(id, "h:1:1");
        assert_eq!(result, Ok(Value::Null));

        let error = ProcessModuleRpcError::new(-32000, "model failed")
            .with_data(json!({"kind":"context_window_exceeded", "limit":128000}));
        let ComponentFrame::Response { id, result } =
            parse_component_frame(json!({"jsonrpc":"2.0", "id":"m:1:2", "error":error}))
                .expect("valid error")
        else {
            panic!("expected response")
        };
        assert_eq!(id, "m:1:2");
        assert_eq!(result, Err(error));
    }

    #[test]
    fn envelopes_reject_malformed_missing_unknown_and_ambiguous_fields() {
        for value in [
            json!(null),
            json!([]),
            json!({"id":"h:1:1", "result":null}),
            json!({"jsonrpc":"1.0", "id":"h:1:1", "result":null}),
            json!({"jsonrpc":2, "id":"h:1:1", "result":null}),
            json!({"jsonrpc":"2.0", "result":null}),
            json!({"jsonrpc":"2.0", "id":1, "result":null}),
            json!({"jsonrpc":"2.0", "id":null, "result":null}),
            json!({"jsonrpc":"2.0", "id":"h:1:1"}),
            json!({"jsonrpc":"2.0", "id":"h:1:1", "method":"search"}),
            json!({"jsonrpc":"2.0", "method":"cancel"}),
            json!({"jsonrpc":"2.0", "id":"h:1:1", "params":null}),
            json!({"jsonrpc":"2.0", "id":"h:1:1", "result":null, "legacy":true}),
            json!({"jsonrpc":"2.0", "id":"h:1:1", "method":"search", "params":null, "legacy":true}),
            json!({"jsonrpc":"2.0", "method":"cancel", "params":null, "legacy":true}),
            json!({"jsonrpc":"2.0", "id":"h:1:1", "result":null, "error":{"code":-1, "message":"failed"}}),
            json!({"jsonrpc":"2.0", "id":"h:1:1", "method":"search", "params":null, "result":null}),
        ] {
            assert!(
                parse_component_frame(value.clone()).is_err(),
                "accepted {value}"
            );
        }
        for error in [
            json!(null),
            json!({"message":"failed"}),
            json!({"code":-1}),
            json!({"code":"-1", "message":"failed"}),
            json!({"code":-1, "message":null}),
            json!({"code":-1, "message":"failed", "legacy":true}),
        ] {
            let value = json!({"jsonrpc":"2.0", "id":"h:1:1", "error":error});
            assert!(
                parse_component_frame(value).is_err(),
                "accepted error {error}"
            );
        }
    }

    #[test]
    fn requests_and_notifications_reject_non_string_or_blank_methods() {
        for method in [json!(null), json!(1), json!(""), json!(" \t\n ")] {
            for has_id in [false, true] {
                let mut value = json!({"jsonrpc":"2.0", "method":method, "params":null});
                if has_id {
                    value["id"] = json!("h:1:1");
                }
                let error = parse_component_frame(value).expect_err("invalid method");
                assert_eq!(
                    error.to_string(),
                    "JSON-RPC method must be a non-empty string"
                );
            }
        }
    }

    #[test]
    fn envelope_decoding_leaves_id_and_method_semantics_to_the_receiver() {
        let ComponentFrame::Request { id, method, params } = parse_component_frame(
            json!({"jsonrpc":"2.0", "id":"not-a-wire-id", "method":" search ", "params":null}),
        )
        .expect("only envelope syntax is checked") else {
            panic!("expected request")
        };
        assert_eq!(id, "not-a-wire-id");
        assert_eq!(method, " search ");
        assert_eq!(params, Value::Null);
    }
}
