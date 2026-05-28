use std::{path::PathBuf, time::SystemTime};

#[derive(Clone)]
pub(crate) struct ResumePicker {
    pub items: Vec<ResumePickerItem>,
    pub selected: usize,
    pub query: String,
}

#[derive(Clone)]
pub(crate) struct ResumePickerItem {
    pub session_dir: PathBuf,
    pub id: String,
    pub created: String,
    pub updated_label: String,
    pub branch: String,
    pub conversation: String,
    pub updated: Option<SystemTime>,
}

impl ResumePicker {
    pub fn new(items: Vec<ResumePickerItem>) -> Self {
        Self {
            items,
            selected: 0,
            query: String::new(),
        }
    }

    pub fn selected_item(&self) -> Option<&ResumePickerItem> {
        self.filtered_items().get(self.selected).copied()
    }

    pub fn move_up(&mut self, by: usize) {
        self.selected = self.selected.saturating_sub(by);
    }

    pub fn move_down(&mut self, by: usize) {
        let len = self.filtered_items().len();
        if len == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected + by).min(len - 1);
    }

    pub fn type_char(&mut self, ch: char) {
        self.query.push(ch);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn filtered_items(&self) -> Vec<&ResumePickerItem> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return self.items.iter().collect();
        }

        self.items
            .iter()
            .filter(|item| {
                item.id.to_lowercase().contains(&query)
                    || item.conversation.to_lowercase().contains(&query)
                    || item.branch.to_lowercase().contains(&query)
            })
            .collect()
    }
}
