use super::request::to_anthropic_request;
use crate::{
    domain::{ImageRef, ModelRef},
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
};

#[test]
fn local_image_is_encoded_as_base64_image_block_before_text() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("image");
    std::fs::write(&path, STANDARD.decode(data).unwrap()).unwrap();
    let request = CanonicalModelRequest::new(
        ModelRef::new("anthropic", "vision"),
        vec![CanonicalMessage::new(
            MessageRole::User,
            vec![
                ContentPart::Image {
                    image: ImageRef {
                        id: "fixture".into(),
                        name: "board.png".into(),
                        mime_type: "image/png".into(),
                        path,
                    },
                },
                ContentPart::Text {
                    text: "Describe".into(),
                },
            ],
        )],
    );
    let body = to_anthropic_request(&request).unwrap();
    assert_eq!(
        body["messages"][0]["content"][0],
        serde_json::json!({"type":"image", "source":{"type":"base64","media_type":"image/png","data":data}})
    );
    assert_eq!(body["messages"][0]["content"][1]["text"], "Describe");
}
