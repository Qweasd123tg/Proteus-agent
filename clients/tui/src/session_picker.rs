use std::path::PathBuf;

#[derive(Clone)]
pub(crate) struct ResumePicker {
    pub items: Vec<ResumePickerItem>,
    pub selected: usize,
}

#[derive(Clone)]
pub(crate) struct ResumePickerItem {
    pub session_dir: PathBuf,
    pub title: String,
    pub detail: String,
}

impl ResumePicker {
    pub fn new(items: Vec<ResumePickerItem>) -> Self {
        Self { items, selected: 0 }
    }

    pub fn selected_item(&self) -> Option<&ResumePickerItem> {
        self.items.get(self.selected)
    }

    pub fn move_up(&mut self, by: usize) {
        self.selected = self.selected.saturating_sub(by);
    }

    pub fn move_down(&mut self, by: usize) {
        if self.items.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected + by).min(self.items.len() - 1);
    }
}
