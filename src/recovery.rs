//! When to stop recovering a crashed display: a few failures in a short time are recovered, a crash
//! loop is not (each recovery would crash again, typically because of an overlay injected into WebView2).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// What to do about a display failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Recover,
    GiveUp,
}

/// Allows at most `max` failures within a sliding `window`; once given up, it stays given up.
#[derive(Debug)]
pub struct RecoveryPolicy {
    max: usize,
    window: Duration,
    failures: VecDeque<Instant>,
    given_up: bool,
}

impl RecoveryPolicy {
    pub fn new(max: usize, window: Duration) -> Self {
        Self { max, window, failures: VecDeque::new(), given_up: false }
    }

    /// Records a failure that happened at `now`.
    pub fn on_failure(&mut self, now: Instant) -> Decision {
        if self.given_up {
            return Decision::GiveUp;
        }
        while self.failures.front().is_some_and(|&first| now.saturating_duration_since(first) > self.window) {
            self.failures.pop_front();
        }
        self.failures.push_back(now);
        if self.failures.len() > self.max {
            self.given_up = true;
            return Decision::GiveUp;
        }
        Decision::Recover
    }

    /// Failures recorded within the window (the last one included).
    pub fn failures(&self) -> usize {
        self.failures.len()
    }
}

/// A rebuild waiting for one of several signals (the old browser process has exited, or a timeout):
/// only the first signal of the current rebuild fires it, so one crash is rebuilt once.
#[derive(Debug, Default)]
pub struct RebuildGate {
    pending: Option<u64>,
}

impl RebuildGate {
    /// Waits for a signal of rebuild `id`, replacing any rebuild still pending.
    pub fn arm(&mut self, id: u64) {
        self.pending = Some(id);
    }

    /// True when the signal of rebuild `id` must trigger it (then the gate closes).
    pub fn fire(&mut self, id: u64) -> bool {
        self.pending.take_if(|pending| *pending == id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rebuild_fires_once_whatever_signals_it() {
        let mut gate = RebuildGate::default();
        assert!(!gate.fire(1), "nothing armed");
        gate.arm(1);
        assert!(gate.fire(1), "browser exited");
        assert!(!gate.fire(1), "the timeout of the same rebuild");
    }

    #[test]
    fn signals_of_an_older_rebuild_are_ignored() {
        let mut gate = RebuildGate::default();
        gate.arm(1);
        gate.arm(2);
        assert!(!gate.fire(1));
        assert!(gate.fire(2));
    }

    const WINDOW: Duration = Duration::from_secs(60);

    #[test]
    fn three_failures_within_the_window_are_recovered_the_fourth_gives_up() {
        let mut policy = RecoveryPolicy::new(3, WINDOW);
        let start = Instant::now();
        for i in 0..3 {
            assert_eq!(policy.on_failure(start + Duration::from_secs(i * 10)), Decision::Recover, "failure {i}");
        }
        assert_eq!(policy.failures(), 3);
        assert_eq!(policy.on_failure(start + Duration::from_secs(30)), Decision::GiveUp);
    }

    #[test]
    fn failures_spaced_beyond_the_window_never_give_up() {
        let mut policy = RecoveryPolicy::new(3, WINDOW);
        let start = Instant::now();
        for i in 0..20 {
            assert_eq!(policy.on_failure(start + Duration::from_secs(i * 61)), Decision::Recover, "failure {i}");
            assert_eq!(policy.failures(), 1);
        }
    }

    #[test]
    fn old_failures_are_forgotten() {
        let mut policy = RecoveryPolicy::new(3, WINDOW);
        let start = Instant::now();
        for i in 0..3 {
            assert_eq!(policy.on_failure(start + Duration::from_secs(i)), Decision::Recover);
        }
        // 70 s later the first three have left the window.
        assert_eq!(policy.on_failure(start + Duration::from_secs(70)), Decision::Recover);
        assert_eq!(policy.failures(), 1);
    }

    #[test]
    fn giving_up_is_final() {
        let mut policy = RecoveryPolicy::new(3, WINDOW);
        let start = Instant::now();
        for i in 0..4 {
            policy.on_failure(start + Duration::from_secs(i));
        }
        assert_eq!(policy.on_failure(start + Duration::from_secs(5)), Decision::GiveUp);
        assert_eq!(policy.on_failure(start + Duration::from_secs(3600)), Decision::GiveUp);
    }
}
