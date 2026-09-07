use super::*;
use crate::model_standard::ContentPart;
use serde_json::json;

#[test]
fn context_part_requires_typed_render_mode_and_preserves_opaque_metadata() {
    for mode in [
        ContextRenderMode::SourceAnnotated,
        ContextRenderMode::Verbatim,
    ] {
        let chunk = ContextChunk::new("external", "exact context\n")
            .with_render_mode(mode)
            .with_metadata(json!({"render_mode": "private diagnostic", "values": [1, 2]}));
        let part = ContentPart::Context { chunk };
        let value = serde_json::to_value(&part).unwrap();
        let decoded: ContentPart = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded, part);

        let mut missing = value.clone();
        missing["Context"]["chunk"]
            .as_object_mut()
            .unwrap()
            .remove("render_mode");
        // The old metadata-only representation cannot silently change the prompt.
        missing["Context"]["chunk"]["metadata"] = json!({"model_visible_render": "verbatim"});
        assert!(
            serde_json::from_value::<ContentPart>(missing)
                .unwrap_err()
                .to_string()
                .contains("render_mode")
        );

        for invalid in [
            json!("verbatin"),
            json!(null),
            json!(false),
            json!({"mode": "verbatim"}),
        ] {
            let mut malformed = value.clone();
            malformed["Context"]["chunk"]["render_mode"] = invalid;
            serde_json::from_value::<ContentPart>(malformed).expect_err("unknown render mode");
        }
    }
}
