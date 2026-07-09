//! Token-бюджет исполнения: единый аккумулятор spend и проверка потолка.
//!
//! Это НЕ slot (см. `docs/slot-governance.md`): одна правдоподобная реализация
//! «считай и остановись», поэтому тип живёт как contract-utility. Первый
//! потребитель — subagent-раннеры (`SubagentLimits::max_total_tokens`),
//! спроектирован под будущие phase/turn-бюджеты workflow.

use serde::{Deserialize, Serialize};

use crate::model_standard::TokenUsage;

/// Аккумулятор фактического token-spend с опциональным потолком.
///
/// Spend = сумма `input + output` по всем model-запросам (реальная модель
/// биллинга: input пересчитывается провайдером на каждом запросе).
/// `None` потолок — безлимит, `exceeded()` всегда `false`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BudgetTracker {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spent: Option<TokenUsage>,
}

impl BudgetTracker {
    pub fn new(max_total_tokens: Option<u64>) -> Self {
        Self {
            max_total_tokens,
            spent: None,
        }
    }

    /// Учитывает usage очередного model-запроса. `None` — запрос без usage
    /// (провайдер не вернул) — не меняет состояние.
    pub fn record(&mut self, usage: Option<&TokenUsage>) {
        let Some(usage) = usage else {
            return;
        };
        match &mut self.spent {
            None => self.spent = Some(usage.clone()),
            Some(total) => total.accumulate(usage),
        }
    }

    /// Накопленный usage (сумма всех записанных запросов).
    pub fn spent(&self) -> Option<&TokenUsage> {
        self.spent.as_ref()
    }

    /// Суммарный spend в токенах: input + output.
    pub fn total_spent_tokens(&self) -> u64 {
        self.spent
            .as_ref()
            .map(TokenUsage::total_tokens)
            .unwrap_or(0)
    }

    /// Потолок бюджета, если задан.
    pub fn max_total_tokens(&self) -> Option<u64> {
        self.max_total_tokens
    }

    /// Превышен ли потолок фактическим spend-ом.
    pub fn exceeded(&self) -> bool {
        match self.max_total_tokens {
            None => false,
            Some(max) => self.total_spent_tokens() > max,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_budget_never_exceeds() {
        let mut tracker = BudgetTracker::new(None);
        tracker.record(Some(&TokenUsage::new(u32::MAX, u32::MAX)));
        assert!(!tracker.exceeded());
        assert_eq!(
            tracker.total_spent_tokens(),
            u64::from(u32::MAX) * 2,
            "spend всё равно учитывается"
        );
    }

    #[test]
    fn record_accumulates_and_trips_threshold() {
        let mut tracker = BudgetTracker::new(Some(1_000));
        tracker.record(Some(&TokenUsage::new(400, 100)));
        assert!(!tracker.exceeded());

        tracker.record(None); // запрос без usage не меняет состояние
        assert_eq!(tracker.total_spent_tokens(), 500);

        tracker.record(Some(&TokenUsage::new(450, 60)));
        assert_eq!(tracker.total_spent_tokens(), 1_010);
        assert!(tracker.exceeded());
    }

    #[test]
    fn exact_limit_is_not_exceeded() {
        let mut tracker = BudgetTracker::new(Some(500));
        tracker.record(Some(&TokenUsage::new(400, 100)));
        assert!(!tracker.exceeded());
    }

    #[test]
    fn accumulate_sums_optional_categories() {
        let mut tracker = BudgetTracker::new(None);
        tracker.record(Some(
            &TokenUsage::new(100, 10).with_cached_input_tokens(Some(40)),
        ));
        tracker.record(Some(
            &TokenUsage::new(200, 20)
                .with_cached_input_tokens(Some(60))
                .with_reasoning_output_tokens(Some(5)),
        ));

        let spent = tracker.spent().expect("spent");
        assert_eq!(spent.input_tokens, 300);
        assert_eq!(spent.output_tokens, 30);
        assert_eq!(spent.cached_input_tokens, Some(100));
        assert_eq!(spent.reasoning_output_tokens, Some(5));
        assert_eq!(spent.cache_creation_input_tokens, None);
    }
}
