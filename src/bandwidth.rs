//! Fake upload speed: one random global rate shared between torrents by a
//! leecher-aware weight. Same algorithm as Joal's `BandwidthDispatcher`.

use std::collections::HashMap;
use std::time::Duration;

use rand::RngExt;

use crate::torrent::InfoHash;

/// Higher weight for torrents with more leechers (Joal `PeersAwareWeightCalculator`).
pub fn weight(seeders: u32, leechers: u32) -> f64 {
    if seeders == 0 || leechers == 0 {
        return 0.0;
    }
    let leechers = f64::from(leechers);
    let ratio = leechers / (f64::from(seeders) + leechers);
    ratio * 100.0 * ratio * leechers
}

pub fn uploaded_during(speed_bps: u64, elapsed: Duration) -> u64 {
    u64::try_from(u128::from(speed_bps) * elapsed.as_millis() / 1000).unwrap_or(u64::MAX)
}

#[derive(Debug)]
pub struct Bandwidth {
    min_bps: u64,
    max_bps: u64,
    global_bps: u64,
    weights: HashMap<InfoHash, f64>,
    speeds: HashMap<InfoHash, u64>,
}

impl Bandwidth {
    pub fn new(min_kbps: u64, max_kbps: u64) -> Self {
        let mut bandwidth = Self {
            min_bps: 0,
            max_bps: 0,
            global_bps: 0,
            weights: HashMap::new(),
            speeds: HashMap::new(),
        };
        bandwidth.set_rates(min_kbps, max_kbps);
        bandwidth
    }

    pub fn set_rates(&mut self, min_kbps: u64, max_kbps: u64) {
        self.min_bps = min_kbps.saturating_mul(1000);
        self.max_bps = max_kbps.saturating_mul(1000);
        self.refresh_global();
    }

    /// Draws a new global speed in `[min, max)` (or `max` when they are equal).
    pub fn refresh_global(&mut self) {
        self.global_bps = if self.min_bps >= self.max_bps {
            self.max_bps
        } else {
            rand::rng().random_range(self.min_bps..self.max_bps)
        };
        self.recompute();
    }

    pub fn global_speed(&self) -> u64 {
        self.global_bps
    }

    /// Sum of the speeds actually assigned to torrents.
    pub fn total_speed(&self) -> u64 {
        self.speeds.values().sum()
    }

    pub fn register(&mut self, info_hash: InfoHash) {
        self.weights.insert(info_hash, 0.0);
        self.recompute();
    }

    pub fn unregister(&mut self, info_hash: &InfoHash) {
        self.weights.remove(info_hash);
        self.speeds.remove(info_hash);
        self.recompute();
    }

    pub fn update_peers(&mut self, info_hash: &InfoHash, seeders: u32, leechers: u32) {
        if let Some(w) = self.weights.get_mut(info_hash) {
            *w = weight(seeders, leechers);
        }
        self.recompute();
    }

    pub fn speed_of(&self, info_hash: &InfoHash) -> u64 {
        self.speeds.get(info_hash).copied().unwrap_or(0)
    }

    pub fn clear(&mut self) {
        self.weights.clear();
        self.speeds.clear();
    }

    fn recompute(&mut self) {
        let total: f64 = self.weights.values().sum();
        let global = self.global_bps as f64;
        self.speeds = self
            .weights
            .iter()
            .map(|(hash, w)| (*hash, if total > 0.0 { (global * w / total) as u64 } else { 0 }))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: InfoHash = [1; 20];
    const B: InfoHash = [2; 20];

    #[test]
    fn weight_follows_joal_formula() {
        assert_eq!(weight(0, 5), 0.0);
        assert_eq!(weight(5, 0), 0.0);
        // ratio = 5 / 10 = 0.5 → 0.5 * 100 * 0.5 * 5
        assert_eq!(weight(5, 5), 125.0);
        assert!(weight(1, 9) > weight(9, 1));
    }

    #[test]
    fn global_speed_is_drawn_in_range() {
        for _ in 0..100 {
            let b = Bandwidth::new(30, 170);
            assert!((30_000..170_000).contains(&b.global_speed()));
        }
        assert_eq!(Bandwidth::new(100, 100).global_speed(), 100_000);
    }

    #[test]
    fn speed_is_split_by_weight() {
        let mut b = Bandwidth::new(100, 100);
        b.register(A);
        b.register(B);
        assert_eq!((b.speed_of(&A), b.speed_of(&B)), (0, 0), "no peers known yet");
        b.update_peers(&A, 5, 5); // 125
        b.update_peers(&B, 1, 9); // 0.9 * 100 * 0.9 * 9 = 729
        let (a, b_speed) = (b.speed_of(&A), b.speed_of(&B));
        assert_eq!(a, (100_000.0 * 125.0 / 854.0) as u64);
        assert_eq!(b_speed, (100_000.0 * 729.0 / 854.0) as u64);
        assert!(b.total_speed() <= 100_000);
    }

    #[test]
    fn unregister_gives_bandwidth_back() {
        let mut b = Bandwidth::new(100, 100);
        b.register(A);
        b.register(B);
        b.update_peers(&A, 5, 5);
        b.update_peers(&B, 5, 5);
        b.unregister(&B);
        assert_eq!(b.speed_of(&A), 100_000);
        assert_eq!(b.speed_of(&B), 0);
        b.clear();
        assert_eq!(b.total_speed(), 0);
    }

    #[test]
    fn set_rates_redraws_and_recomputes() {
        let mut b = Bandwidth::new(100, 100);
        b.register(A);
        b.update_peers(&A, 1, 1);
        b.set_rates(50, 50);
        assert_eq!(b.speed_of(&A), 50_000);
    }

    #[test]
    fn uploaded_during_uses_elapsed_time() {
        assert_eq!(uploaded_during(1000, Duration::from_millis(1500)), 1500);
        assert_eq!(uploaded_during(0, Duration::from_secs(10)), 0);
    }
}
