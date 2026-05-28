use crate::visual::VisualMessage;

#[derive(Clone)]
pub(crate) enum ActiveCell {
    AssistantStreaming(VisualMessage),
}

#[derive(Clone)]
pub(crate) struct TranscriptStore {
    committed: Vec<VisualMessage>,
    emitted_until: usize,
    active: Option<ActiveCell>,
}

impl TranscriptStore {
    pub(crate) fn new(committed: Vec<VisualMessage>) -> Self {
        Self {
            committed,
            emitted_until: 0,
            active: None,
        }
    }

    pub(crate) fn push_committed(&mut self, message: VisualMessage) {
        self.committed.push(message);
    }

    pub(crate) fn clear_committed(&mut self) {
        self.committed.clear();
        self.emitted_until = 0;
    }

    pub(crate) fn clear_active(&mut self) {
        self.active = None;
    }

    pub(crate) fn append_committed(&mut self, messages: &mut Vec<VisualMessage>) {
        self.committed.append(messages);
    }

    pub(crate) fn committed(&self) -> &[VisualMessage] {
        &self.committed
    }

    pub(crate) fn active_message(&self) -> Option<&VisualMessage> {
        match self.active.as_ref()? {
            ActiveCell::AssistantStreaming(message) => Some(message),
        }
    }

    pub(crate) fn is_streaming(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn append_active_assistant(&mut self, chunk: &str) {
        match &mut self.active {
            Some(ActiveCell::AssistantStreaming(message)) => message.text.push_str(chunk),
            None => {
                self.active = Some(ActiveCell::AssistantStreaming(VisualMessage::assistant(
                    chunk,
                )));
            }
        }
    }

    pub(crate) fn finalize_active_assistant(&mut self, final_text: String) {
        self.active = None;
        self.committed.push(VisualMessage::assistant(final_text));
    }

    pub(crate) fn commit_active_assistant(&mut self) {
        let Some(ActiveCell::AssistantStreaming(message)) = self.active.take() else {
            return;
        };
        self.committed.push(message);
    }

    pub(crate) fn draft_active_assistant(&mut self) {
        let Some(ActiveCell::AssistantStreaming(mut message)) = self.active.take() else {
            return;
        };
        message.role = crate::visual::VisualRole::Draft;
        self.committed.push(message);
    }

    pub(crate) fn reset_emitted(&mut self) {
        self.emitted_until = 0;
    }

    pub(crate) fn drain_new_messages(&mut self) -> Vec<VisualMessage> {
        let mut drained = Vec::new();
        while let Some(message) = self.committed.get(self.emitted_until) {
            drained.push(message.clone());
            self.emitted_until += 1;
        }
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::{ToolCard, ToolStatus};

    #[test]
    fn active_assistant_is_not_emitted_until_committed() {
        let mut transcript = TranscriptStore::new(vec![VisualMessage::system("ready")]);
        transcript.append_active_assistant("partial");

        let drained = transcript.drain_new_messages();

        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].text, "ready");
        assert!(transcript.is_streaming());

        transcript.finalize_active_assistant("final".to_owned());
        let drained = transcript.drain_new_messages();

        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].text, "final");
    }

    #[test]
    fn running_tool_is_append_only_and_does_not_block_later_emission() {
        let mut transcript = TranscriptStore::new(vec![VisualMessage::tool(ToolCard {
            call_id: "call-1".to_owned(),
            name: "rg".to_owned(),
            args_summary: "rg foo".to_owned(),
            status: ToolStatus::Running,
            output_preview: String::new(),
        })]);
        transcript.push_committed(VisualMessage::assistant("later"));

        let drained = transcript.drain_new_messages();

        assert_eq!(drained.len(), 2);
        assert_eq!(
            drained[0].tool.as_ref().unwrap().status,
            ToolStatus::Running
        );
        assert_eq!(drained[1].text, "later");
    }
}
