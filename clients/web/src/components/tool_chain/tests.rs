use super::*;
use crate::types::{MessageRole, ToolActivity};
fn message(id: u64, tool: Option<ToolActivityStatus>) -> Message {
    Message {
        id,
        message_id: None,
        phase: None,
        version: 0,
        text_offset: 0,
        role: MessageRole::Assistant,
        text: "Комментарий, который нельзя скрыть".into(),
        subagent: None,
        streaming: false,
        tool: tool.map(|status| ToolActivity {
            call_id: id.to_string(),
            name: "shell".into(),
            args: serde_json::json!({}),
            args_preview: String::new(),
            started_at_ms: 0,
            finished_at_ms: None,
            status,
            result_preview: None,
        }),
    }
}
#[test]
fn comments_split_chains_and_attention_remains_in_summary() {
    use ToolActivityStatus::*;
    let mut transcript = vec![
        message(1, None),
        message(2, Some(Done)),
        message(3, Some(WaitingApproval)),
        message(4, None),
        message(5, Some(Failed)),
        message(6, Some(Interrupted)),
    ];
    let initial = groups(&transcript);
    assert_eq!(
        initial.iter().map(|g| g.ids.clone()).collect::<Vec<_>>(),
        vec![vec![1], vec![2, 3], vec![4], vec![5, 6]]
    );
    assert!(initial[1].label().contains("ждут разрешения: 1"));
    assert!(initial[3].label().contains("ошибок/отказов: 1"));
    assert!(initial[3].label().contains("прервано: 1"));
    transcript[2].tool.as_mut().unwrap().status = Done;
    transcript.push(message(7, Some(Running)));
    let next = groups(&transcript);
    assert_eq!(
        next.iter().map(|g| (g.id, g.tools)).collect::<Vec<_>>(),
        initial.iter().map(|g| (g.id, g.tools)).collect::<Vec<_>>()
    );
    assert!(!next[1].label().contains("ждут разрешения"));
    assert!(next[3].label().contains("выполняются: 1"));
}
