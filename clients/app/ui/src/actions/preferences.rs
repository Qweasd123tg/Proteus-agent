use super::AppActions;
impl AppActions {
    pub(super) async fn remember_selection(self, session: &str, generation: u64) {
        let result = crate::model_preference::read_current(session).await;
        if !self.is_current_session(session, generation) {
            return;
        }
        if let Err(error) =
            result.and_then(|selection| crate::model_preference::remember(&selection))
        {
            self.set_control_error("Не удалось запомнить модель и effort", error);
        }
    }
}
