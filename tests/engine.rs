mod common;

use std::time::{Duration, Instant};

use common::*;
use ratiod::engine::Status;

fn seeding(h: &Harness) -> bool {
    h.engine.snapshot().torrents.first().is_some_and(|t| t.status == Status::Seeding)
}

#[tokio::test]
async fn announces_started_then_regular_with_growing_uploaded() {
    let tracker = FakeTracker::start(ok(1, 5, 5)).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 10_000_000_000, &tracker))]).await;
    h.engine.start();
    wait_for("two announces", || tracker.requests().len() >= 2).await;
    let reqs = tracker.requests();
    assert_eq!(reqs[0].get("event").map(String::as_str), Some("started"));
    assert_eq!(reqs[0]["uploaded"], "0");
    assert_eq!(reqs[0]["downloaded"], "0");
    assert_eq!(reqs[0]["left"], "0");
    assert_eq!(reqs[0]["port"], "49200");
    assert!(!reqs[1].contains_key("event"), "a regular announce has no event");
    assert!(reqs[1]["uploaded"].parse::<u64>().unwrap() > 0);
    let snap = h.engine.snapshot();
    assert!(snap.running);
    assert_eq!(snap.torrents[0].seeders, Some(4));
    assert_eq!(snap.torrents[0].leechers, Some(5));
    assert!(snap.session_uploaded > 0);
}

#[tokio::test]
async fn ratio_reached_archives_and_sends_stopped() {
    let tracker = FakeTracker::start(ok(1, 5, 5)).await;
    let mut config = test_config();
    config.upload_ratio_target = 1.0;
    let h = harness(config, &[("small.torrent", torrent("small", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("archived file", || h.library.archive_dir().join("small.torrent").exists()).await;
    wait_for("stopped announce", || tracker.events().last().is_some_and(|e| e == "stopped")).await;
    assert!(!h.library.torrents_dir().join("small.torrent").exists());
    wait_for("empty snapshot", || h.engine.snapshot().torrents.is_empty()).await;
}

#[tokio::test]
async fn zero_leechers_archived_when_not_kept() {
    let tracker = FakeTracker::start(ok(60, 5, 0)).await;
    let mut config = test_config();
    config.keep_torrent_with_zero_leechers = false;
    let h = harness(config, &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("archived", || h.library.archive_dir().join("a.torrent").exists()).await;
    wait_for("started then stopped", || tracker.events() == ["started", "stopped"]).await;
}

#[tokio::test]
async fn zero_leechers_kept_by_default() {
    let tracker = FakeTracker::start(ok(60, 5, 0)).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("seeding", || seeding(&h)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(h.library.torrents_dir().join("a.torrent").exists());
    assert_eq!(tracker.events(), ["started"]);
}

#[tokio::test]
async fn five_failures_archive_without_stopped() {
    let tracker = FakeTracker::start(Behavior::Fail).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("archived", || h.library.archive_dir().join("a.torrent").exists()).await;
    assert_eq!(tracker.events(), ["started"; 5]);
    wait_for("empty snapshot", || h.engine.snapshot().torrents.is_empty()).await;
}

#[tokio::test]
async fn failures_are_visible_in_snapshot() {
    let tracker = FakeTracker::start(Behavior::Fail).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("an error", || {
        h.engine.snapshot().torrents.first().is_some_and(|t| t.consecutive_fails >= 1 && t.last_error.is_some())
    })
    .await;
    let error = h.engine.snapshot().torrents[0].last_error.clone().unwrap();
    assert!(error.contains("HTTP 500"), "{error}");
}

#[tokio::test]
async fn stop_finishes_even_if_tracker_hangs() {
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("seeding", || seeding(&h)).await;
    tracker.set(Behavior::Hang);
    let began = Instant::now();
    h.engine.stop().await;
    assert!(began.elapsed() < Duration::from_secs(2), "stop took {:?}", began.elapsed());
    assert_eq!(tracker.events(), ["started", "stopped"]);
    let snap = h.engine.snapshot();
    assert!(!snap.running);
    assert!(snap.torrents.is_empty());
    assert_eq!(snap.queued, 1);
}

#[tokio::test]
async fn respects_simultaneous_seed_and_promotes_queue_on_delete() {
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let mut config = test_config();
    config.simultaneous_seed = 1;
    let h =
        harness(config, &[("a.torrent", torrent("a", 1000, &tracker)), ("b.torrent", torrent("b", 1000, &tracker))])
            .await;
    h.engine.start();
    wait_for("a seeding", || seeding(&h)).await;
    let snap = h.engine.snapshot();
    assert_eq!((snap.torrents.len(), snap.queued), (1, 1));
    assert_eq!(snap.torrents[0].name, "a");

    assert!(h.engine.delete(hash_from_hex(&snap.torrents[0].info_hash)).await);
    assert!(!h.library.torrents_dir().join("a.torrent").exists());
    wait_for("b promoted", || h.engine.snapshot().torrents.first().is_some_and(|t| t.name == "b")).await;
    assert_eq!(h.engine.snapshot().queued, 0);
    wait_for("stopped for a", || tracker.events().contains(&"stopped".to_owned())).await;
    assert!(!h.engine.delete([9; 20]).await);
}

#[tokio::test]
async fn duplicate_file_is_not_seeded_twice() {
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let bytes = torrent("a", 1000, &tracker);
    let h = harness(test_config(), &[]).await;
    h.engine.start();
    std::fs::write(h.library.torrents_dir().join("a.torrent"), &bytes).unwrap();
    std::fs::write(h.library.torrents_dir().join("a-copy.torrent"), &bytes).unwrap();
    h.engine.rescan();
    wait_for("a seeding", || seeding(&h)).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let snap = h.engine.snapshot();
    assert_eq!((snap.torrents.len(), snap.queued), (1, 0));
    assert_eq!(tracker.events(), ["started"]);
}

#[tokio::test]
async fn removed_file_is_stopped_on_rescan() {
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("seeding", || seeding(&h)).await;
    std::fs::remove_file(h.library.torrents_dir().join("a.torrent")).unwrap();
    h.engine.rescan();
    wait_for("stopped", || tracker.events() == ["started", "stopped"]).await;
    assert!(h.engine.snapshot().torrents.is_empty());
}

#[tokio::test]
async fn invalid_config_is_rejected_and_seeding_continues() {
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("seeding", || seeding(&h)).await;
    let mut bad = test_config();
    bad.client = "nope.client".into();
    let error = h.engine.update_config(bad).await.unwrap_err();
    assert_eq!(error.field, "client");
    assert!(!h.dir.path().join("config.json").exists(), "nothing written");
    assert_eq!(tracker.events(), ["started"], "no restart");
    assert!(h.engine.snapshot().running);
}

#[tokio::test]
async fn valid_config_change_is_saved_and_restarts_seeding() {
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("seeding", || seeding(&h)).await;
    let mut next = test_config();
    next.client = "transmission-3.00.client".into();
    h.engine.update_config(next.clone()).await.unwrap();
    let saved: ratiod::config::AppConfig =
        serde_json::from_slice(&std::fs::read(h.dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(saved, next);
    assert_eq!(h.engine.snapshot().client, "transmission-3.00.client");
    wait_for("restart", || tracker.events() == ["started", "stopped", "started"]).await;
}

#[tokio::test]
async fn fails_over_to_next_tracker() {
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://{}/announce", listener.local_addr().unwrap())
    };
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let bytes = ratiod::testutil::torrent_with_trackers("a", 1000, &[&dead, &tracker.url]);
    let h = harness(test_config(), &[("a.torrent", bytes)]).await;
    h.engine.start();
    wait_for("seeding via the second tracker", || seeding(&h)).await;
    assert_eq!(tracker.events(), ["started"]);
}

#[tokio::test]
async fn unknown_client_reports_an_error_instead_of_starting() {
    let mut config = test_config();
    config.client = "nope.client".into();
    let h = harness(config, &[]).await;
    h.engine.start();
    wait_for("an error", || h.engine.snapshot().error.is_some()).await;
    assert!(!h.engine.snapshot().running);
}

#[tokio::test]
async fn shutdown_sends_stopped() {
    let tracker = FakeTracker::start(ok(60, 5, 5)).await;
    let h = harness(test_config(), &[("a.torrent", torrent("a", 1000, &tracker))]).await;
    h.engine.start();
    wait_for("seeding", || seeding(&h)).await;
    h.engine.shutdown().await;
    assert_eq!(tracker.events(), ["started", "stopped"]);
}
