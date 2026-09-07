use std::collections::BTreeMap;

/// Exact duplicate detection without retaining one allocation per stream event.
/// Adjacent ids collapse into ranges; arbitrary out-of-order ids remain supported.
#[derive(Default)]
pub(super) struct CallbackIds(BTreeMap<u64, u64>);

#[derive(Debug, PartialEq)]
pub(super) enum InsertError {
    Reused,
    Capacity,
}

impl CallbackIds {
    pub fn insert(&mut self, id: u64, max_ranges: usize) -> Result<(), InsertError> {
        let left = self.0.range(..=id).next_back().map(|(&a, &b)| (a, b));
        if left.is_some_and(|(_, end)| id <= end) {
            return Err(InsertError::Reused);
        }
        let right = self.0.range(id..).next().map(|(&a, &b)| (a, b));
        let joins_left = left.is_some_and(|(_, end)| end.checked_add(1) == Some(id));
        let joins_right = right.is_some_and(|(start, _)| id.checked_add(1) == Some(start));
        if !joins_left && !joins_right && self.0.len() >= max_ranges {
            return Err(InsertError::Capacity);
        }
        let start = if joins_left { left.unwrap().0 } else { id };
        let end = if joins_right {
            let (start, end) = right.unwrap();
            self.0.remove(&start);
            end
        } else {
            id
        };
        self.0.insert(start, end);
        Ok(())
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_sequential_stream_uses_one_range_and_never_forgets_ids() {
        let mut ids = CallbackIds::default();
        for id in 1..=100_000 {
            ids.insert(id, 1).unwrap();
        }
        assert_eq!(ids.0.len(), 1);
        for id in [1, 500, 100_000] {
            assert_eq!(ids.insert(id, 1), Err(InsertError::Reused));
        }
    }
    #[test]
    fn sparse_ids_are_bounded_and_gap_closure_merges_ranges() {
        let mut ids = CallbackIds::default();
        ids.insert(3, 2).unwrap();
        ids.insert(1, 2).unwrap();
        assert_eq!(ids.insert(5, 2), Err(InsertError::Capacity));
        ids.insert(2, 2).unwrap();
        ids.insert(5, 2).unwrap();
        ids.insert(4, 2).unwrap();
        assert_eq!(ids.0.into_iter().collect::<Vec<_>>(), vec![(1, 5)]);
    }
    #[test]
    fn maximum_id_does_not_wrap() {
        let mut ids = CallbackIds::default();
        ids.insert(u64::MAX, 2).unwrap();
        ids.insert(u64::MAX - 1, 2).unwrap();
        assert_eq!(ids.insert(u64::MAX, 2), Err(InsertError::Reused));
        ids.clear();
        ids.insert(1, 2).unwrap();
    }
}
