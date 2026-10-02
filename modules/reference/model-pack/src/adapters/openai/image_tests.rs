use super::request::to_openai_request;
use crate::{
    domain::{ImageRef, ModelRef},
    model_standard::{CanonicalMessage, CanonicalModelRequest, ContentPart, MessageRole},
};

#[test]
fn local_image_is_encoded_as_image_input_and_missing_file_fails_explicitly() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("image");
    std::fs::write(&path, STANDARD.decode(data).unwrap()).unwrap();
    let request = CanonicalModelRequest::new(
        ModelRef::new("openai", "vision"),
        vec![CanonicalMessage::new(
            MessageRole::User,
            vec![
                ContentPart::Image {
                    image: ImageRef {
                        id: "fixture".into(),
                        name: "board.png".into(),
                        mime_type: "image/png".into(),
                        path: path.clone(),
                    },
                },
                ContentPart::Text {
                    text: "Describe".into(),
                },
            ],
        )],
    );
    let body = to_openai_request(&request).unwrap();
    assert_eq!(body["input"][0]["content"][0]["type"], "input_image");
    assert_eq!(
        body["input"][0]["content"][0]["image_url"],
        format!("data:image/png;base64,{data}")
    );
    assert_eq!(body["input"][0]["content"][1]["text"], "Describe");
    std::fs::remove_file(path).unwrap();
    assert!(
        format!("{:#}", to_openai_request(&request).unwrap_err())
            .contains("cannot read attached image")
    );
}
