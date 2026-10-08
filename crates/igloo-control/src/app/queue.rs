use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::time::Duration;

use tokio::time::Instant;

/// Keys waiting to be processed: each at most once in the ready queue, with delayed retries
/// and exponential backoff per key.
pub(crate) struct WorkQueue<K> {
    ready: VecDeque<K>,
    queued: HashSet<K>,
    delayed: BTreeSet<(Instant, K)>,
    attempts: HashMap<K, u32>,
    backoff: Backoff,
}

/// Exponential backoff bounds.
#[derive(Clone, Copy, Debug)]
pub struct Backoff {
    /// The delay after the first failure.
    pub initial: Duration,
    /// The longest delay.
    pub max: Duration,
}

impl<K: Copy + Eq + Hash + Ord> WorkQueue<K> {
    pub(crate) fn new(backoff: Backoff) -> Self {
        Self {
            ready: VecDeque::new(),
            queued: HashSet::new(),
            delayed: BTreeSet::new(),
            attempts: HashMap::new(),
            backoff,
        }
    }

    /// Queues `key` now, unless it is already waiting.
    pub(crate) fn push(&mut self, key: K) {
        if self.queued.insert(key) {
            self.ready.push_back(key);
        }
    }

    /// Queues `key` once `at` has passed.
    pub(crate) fn push_at(&mut self, key: K, at: Instant) {
        self.delayed.insert((at, key));
    }

    /// Queues `key` after its next backoff delay.
    pub(crate) fn retry(&mut self, key: K, now: Instant) {
        let attempts = self.attempts.entry(key).or_insert(0);
        *attempts = attempts.saturating_add(1);
        let exponent = attempts.saturating_sub(1).min(16);
        let delay = self
            .backoff
            .initial
            .saturating_mul(1 << exponent)
            .min(self.backoff.max);
        self.push_at(key, now + delay);
    }

    /// Clears the failure count of `key` after a success.
    pub(crate) fn succeeded(&mut self, key: K) {
        self.attempts.remove(&key);
    }

    /// The next key ready at `now`, promoting delayed keys whose time has come.
    pub(crate) fn pop(&mut self, now: Instant) -> Option<K> {
        while let Some(&(at, key)) = self.delayed.first() {
            if at > now {
                break;
            }
            self.delayed.pop_first();
            self.push(key);
        }
        let key = self.ready.pop_front()?;
        self.queued.remove(&key);
        Some(key)
    }

    /// When the earliest delayed key becomes ready.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.delayed.first().map(|(at, _)| *at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> WorkQueue<u32> {
        WorkQueue::new(Backoff {
            initial: Duration::from_secs(1),
            max: Duration::from_secs(4),
        })
    }

    #[test]
    fn deduplicates_ready_keys() {
        let mut queue = queue();
        let now = Instant::now();
        queue.push(1);
        queue.push(1);
        queue.push(2);
        assert_eq!(queue.pop(now), Some(1));
        assert_eq!(queue.pop(now), Some(2));
        assert_eq!(queue.pop(now), None);
    }

    #[test]
    fn delayed_keys_wait_for_their_time() {
        let mut queue = queue();
        let now = Instant::now();
        queue.push_at(1, now + Duration::from_secs(5));
        assert_eq!(queue.pop(now), None);
        assert_eq!(queue.next_deadline(), Some(now + Duration::from_secs(5)));
        assert_eq!(queue.pop(now + Duration::from_secs(5)), Some(1));
    }

    #[test]
    fn retries_back_off_exponentially_up_to_the_max() {
        let mut queue = queue();
        let now = Instant::now();
        let delays: Vec<Duration> = (0..4)
            .map(|_| {
                queue.retry(1, now);
                let deadline = queue.next_deadline().expect("delayed");
                queue.pop(deadline);
                deadline - now
            })
            .collect();
        let secs: Vec<u64> = delays.iter().map(Duration::as_secs).collect();
        assert_eq!(secs, [1, 2, 4, 4]);
        queue.succeeded(1);
        queue.retry(1, now);
        assert_eq!(queue.next_deadline(), Some(now + Duration::from_secs(1)));
    }
}
