use crate::model_standard::{CanonicalMessage, ContentPart, MessageRole};

pub(super) fn reconcile_current_user_message(
    messages: &mut [CanonicalMessage],
    new_messages_start: usize,
    persisted_user_message: &CanonicalMessage,
) -> bool {
    let Some(message) = messages.get_mut(new_messages_start) else {
        return false;
    };
    if !same_user_text(message, persisted_user_message) {
        return false;
    }
    *message = persisted_user_message.clone();
    true
}

fn same_user_text(left: &CanonicalMessage, right: &CanonicalMessage) -> bool {
    left.role == MessageRole::User
        && right.role == MessageRole::User
        && canonical_text(left) == canonical_text(right)
}

fn canonical_text(message: &CanonicalMessage) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}
