use std::collections::HashSet;
use std::hash::Hash;

/// A reusable activity scheduler.
///
/// Tracks a set of "active" item keys for any system that wants to update (tick)
/// only a subset of its items rather than everything every frame. Systems such as
/// tile entities, mobs, machines, and weather can each hold one of these keyed by
/// their own domain (block coordinate index, entity id, ...) and only process the
/// active subset. Keeping the hot path to active items is the core scalability win.
///
/// The scheduler is deliberately small and generic — it stores opaque keys and
/// knows nothing about what they mean. Callers map their domain to keys.
#[derive(Default, Clone)]
pub struct Scheduler<K> {
    active: HashSet<K>,
}

impl<K> Scheduler<K>
where
    K: Eq + Hash + Copy,
{
    /// Creates an empty scheduler.
    #[must_use]
    pub fn new() -> Self {
        Self { active: HashSet::new() }
    }

    /// Marks a key as active so it will be processed by the owning system.
    pub fn activate(&mut self, key: K) {
        self.active.insert(key);
    }

    /// Marks a key as inactive so it will no longer be processed.
    pub fn deactivate(&mut self, key: K) {
        self.active.remove(&key);
    }

    /// Whether the given key is currently active.
    #[must_use]
    pub fn is_active(&self, key: &K) -> bool {
        self.active.contains(key)
    }

    /// Iterates over all currently-active keys.
    pub fn active(&self) -> impl Iterator<Item = K> + '_ {
        self.active.iter().copied()
    }

    /// Number of active keys.
    #[must_use]
    pub fn num_active(&self) -> usize {
        self.active.len()
    }

    /// Deactivates and clears all keys.
    pub fn clear(&mut self) {
        self.active.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::Scheduler;

    #[test]
    fn starts_empty() {
        let s = Scheduler::<u64>::new();
        assert_eq!(s.num_active(), 0);
        assert_eq!(s.active().count(), 0);
    }

    #[test]
    fn activate_and_query() {
        let mut s = Scheduler::<u64>::new();
        assert!(!s.is_active(&7));
        s.activate(7);
        assert!(s.is_active(&7));
        assert_eq!(s.active().collect::<Vec<_>>(), vec![7]);
    }

    #[test]
    fn activate_is_idempotent() {
        let mut s = Scheduler::<u64>::new();
        s.activate(3);
        s.activate(3);
        assert_eq!(s.num_active(), 1);
    }

    #[test]
    fn deactivate_removes() {
        let mut s = Scheduler::<u64>::new();
        s.activate(1);
        s.activate(2);
        s.deactivate(1);
        assert!(!s.is_active(&1));
        assert!(s.is_active(&2));
        assert_eq!(s.num_active(), 1);
    }

    #[test]
    fn clear_empties() {
        let mut s = Scheduler::<i32>::new();
        s.activate(5);
        s.activate(9);
        s.clear();
        assert_eq!(s.num_active(), 0);
    }

    #[test]
    fn supports_any_hashable_key() {
        // keys are opaque; the scheduler doesn't care what they are
        let mut s = Scheduler::<(u32, u32)>::new();
        s.activate((10, 20));
        assert!(s.is_active(&(10, 20)));
        assert!(!s.is_active(&(10, 21)));
    }
}
