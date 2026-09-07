use super::*;

#[test]
fn sse_output_wraps_protocol_output_as_json_data() {
    let bytes = encode_sse_output(&StdioOutput::Response {
        id: Some("req-1".to_owned()),
        ok: true,
        output: None,
        error: None,
    });
    let text = std::str::from_utf8(&bytes).expect("utf8");

    assert!(text.starts_with("event: output\ndata: "));
    assert!(text.contains(r#""type":"response""#));
    assert!(text.contains(r#""id":"req-1""#));
    assert!(text.ends_with("\n\n"));
}
