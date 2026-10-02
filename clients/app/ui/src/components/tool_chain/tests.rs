use super::*;
use crate::types::{MessageRole, ToolActivity};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn message(id: u64, tool: Option<ToolActivityStatus>) -> Message {
    Message {
        images: Vec::new(),
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
fn visible_chain_status_updates_without_rebuilding_structure() {
    let owner = Owner::new();
    owner.with(|| {
        let mut items = (0..2000).map(|id| message(id, None)).collect::<Vec<_>>();
        items[100] = message(100, Some(ToolActivityStatus::Running));
        items[101] = message(101, Some(ToolActivityStatus::Done));
        items[1900] = message(1900, Some(ToolActivityStatus::Failed));
        let (read, write) = crate::transcript::transcript(items);
        let scans = Arc::new(AtomicUsize::new(0));
        let first_reads = Arc::new(AtomicUsize::new(0));
        let last_reads = Arc::new(AtomicUsize::new(0));
        let groups = Memo::new({
            let scans = scans.clone();
            move |_| {
                read.with_group_structure(|items| {
                    scans.fetch_add(1, Ordering::Relaxed);
                    groups(items)
                })
            }
        });
        let first = Memo::new({
            let first_reads = first_reads.clone();
            move |_| {
                groups.with(|groups| {
                    let chain = groups.iter().find(|group| group.id == 100).unwrap();
                    first_reads.fetch_add(1, Ordering::Relaxed);
                    read.with_tool_statuses(&chain.ids, Summary::from_statuses)
                })
            }
        });
        let last = Memo::new({
            let last_reads = last_reads.clone();
            move |_| {
                groups.with(|groups| {
                    let chain = groups.iter().find(|group| group.id == 1900).unwrap();
                    last_reads.fetch_add(1, Ordering::Relaxed);
                    read.with_tool_statuses(&chain.ids, Summary::from_statuses)
                })
            }
        });
        assert_eq!(first.get().running, 1);
        assert_eq!(last.get().failed, 1);
        assert_eq!(scans.load(Ordering::Relaxed), 1);
        assert_eq!(first_reads.load(Ordering::Relaxed), 1);
        assert_eq!(last_reads.load(Ordering::Relaxed), 1);
        write.update_matching(
            |item| item.id == 100,
            |item| {
                item.tool.as_mut().unwrap().status = ToolActivityStatus::WaitingApproval;
                item.version += 1;
            },
        );
        assert_eq!(first.get().waiting, 1);
        assert_eq!(first.get().running, 0);
        assert_eq!(last.get().failed, 1);
        assert_eq!(scans.load(Ordering::Relaxed), 1);
        assert_eq!(first_reads.load(Ordering::Relaxed), 2);
        assert_eq!(last_reads.load(Ordering::Relaxed), 1);
        write.update_matching(
            |item| item.id == 100,
            |item| {
                item.tool.as_mut().unwrap().result_preview = Some("large output".repeat(100));
                item.version += 1;
            },
        );
        assert_eq!(first.get().waiting, 1);
        assert_eq!(scans.load(Ordering::Relaxed), 1);
        assert_eq!(first_reads.load(Ordering::Relaxed), 2);
        write.update(|items| {
            items[101].tool.as_mut().unwrap().status = ToolActivityStatus::Denied;
            items[101].version += 1;
        });
        assert_eq!(first.get().failed, 1);
        assert_eq!(scans.load(Ordering::Relaxed), 1);
    });
    owner.cleanup();
}

#[test]
fn same_id_snapshot_and_reclassification_rebuild_chains() {
    let owner = Owner::new();
    owner.with(|| {
        let (read, write) = crate::transcript::transcript(vec![
            message(1, Some(ToolActivityStatus::Running)),
            message(2, Some(ToolActivityStatus::Done)),
        ]);
        let structure = Memo::new(move |_| read.with_group_structure(|items| groups(items)));
        assert_eq!(structure.get()[0].ids, [1, 2]);
        write.set(vec![
            message(1, Some(ToolActivityStatus::Failed)),
            message(2, Some(ToolActivityStatus::Done)),
        ]);
        assert_eq!(structure.get()[0].ids, [1, 2]);
        assert_eq!(
            read.with_tool_statuses(&[1, 2], Summary::from_statuses)
                .failed,
            1
        );
        write.update_matching(
            |item| item.id == 2,
            |item| {
                item.tool = None;
                item.version += 1;
            },
        );
        assert_eq!(
            structure
                .get()
                .iter()
                .map(|group| group.ids.clone())
                .collect::<Vec<_>>(),
            vec![vec![1], vec![2]]
        );
        write.update(|items| {
            let item = items.iter_mut().find(|item| item.id == 2).unwrap();
            item.tool = Some(message(2, Some(ToolActivityStatus::Done)).tool.unwrap());
            item.version += 1;
        });
        assert_eq!(structure.get()[0].ids, [1, 2]);
        let mut subagent = message(2, Some(ToolActivityStatus::Done));
        subagent.subagent = Some(crate::types::SubagentActivity {
            child_thread_id: "child".into(),
            role: "reviewer".into(),
            description: None,
            status: crate::types::SubagentActivityStatus::Running,
            iterations: None,
            started_at_ms: 1,
            finished_at_ms: None,
            tools: Vec::new(),
        });
        write.set(vec![message(1, Some(ToolActivityStatus::Failed)), subagent]);
        assert_eq!(
            structure
                .get()
                .iter()
                .map(|group| group.ids.clone())
                .collect::<Vec<_>>(),
            vec![vec![1], vec![2]]
        );
        write.update_matching(
            |item| item.id == 2,
            |item| {
                item.subagent = None;
                item.version += 1;
            },
        );
        assert_eq!(structure.get()[0].ids, [1, 2]);
        write.update_matching(
            |item| item.id == 2,
            |item| {
                item.role = MessageRole::User;
                item.version += 1;
            },
        );
        assert_eq!(structure.get()[0].ids, [1, 2]);
        write.update_matching(
            |item| item.id == 1,
            |item| {
                item.role = MessageRole::User;
                item.version += 1;
            },
        );
        assert_eq!(structure.get()[0].role, MessageRole::User);
        write.update(|items| items.swap(0, 1));
        assert_eq!(structure.get()[0].id, 2);
        write.update(|items| {
            items.remove(0);
        });
        assert_eq!(structure.get()[0].id, 1);
    });
    owner.cleanup();
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
    let summary = |group: &Group, items: &[Message]| {
        Summary::from_statuses(
            &group
                .ids
                .iter()
                .map(|id| {
                    items
                        .iter()
                        .find(|item| item.id == *id)
                        .and_then(|item| item.tool.as_ref().map(|tool| tool.status))
                })
                .collect::<Vec<_>>(),
        )
    };
    assert!(
        summary(&initial[1], &transcript)
            .label()
            .contains("ждут разрешения: 1")
    );
    assert!(
        summary(&initial[3], &transcript)
            .label()
            .contains("ошибок/отказов: 1")
    );
    assert!(
        summary(&initial[3], &transcript)
            .label()
            .contains("прервано: 1")
    );
    transcript[2].tool.as_mut().unwrap().status = Done;
    transcript.push(message(7, Some(Running)));
    let next = groups(&transcript);
    assert_eq!(
        next.iter().map(|g| (g.id, g.tools)).collect::<Vec<_>>(),
        initial.iter().map(|g| (g.id, g.tools)).collect::<Vec<_>>()
    );
    assert!(
        !summary(&next[1], &transcript)
            .label()
            .contains("ждут разрешения")
    );
    assert!(
        summary(&next[3], &transcript)
            .label()
            .contains("выполняются: 1")
    );
}
