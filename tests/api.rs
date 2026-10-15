mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

use common::*;
use ratiod::api::{self, AppState};

fn app(h: &Harness) -> Router {
    app_with(h, None)
}

fn app_with(h: &Harness, on_show: Option<std::sync::Arc<dyn Fn() + Send + Sync>>) -> Router {
    api::router(AppState {
        engine: h.engine.clone(),
        library: h.library.clone(),
        clients_dir: h.dir.path().join("clients"),
        on_show,
    })
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::get(uri).body(Body::empty()).unwrap()
}

fn put_json(uri: &str, value: &serde_json::Value) -> Request<Body> {
    Request::put(uri).header(header::CONTENT_TYPE, "application/json").body(Body::from(value.to_string())).unwrap()
}

fn multipart(files: &[(&str, &[u8])]) -> Request<Body> {
    let boundary = "ratiodtestboundary";
    let mut body = Vec::new();
    for (name, bytes) in files {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"{name}\"\r\nContent-Type: application/x-bittorrent\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Request::post("/api/torrents")
        .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn upload_reports_each_file_and_queues_new_torrents() {
    let h = harness(test_config(), &[]).await;
    let good = ratiod::testutil::torrent_file("good", 10, "http://t/announce");
    let response = app(&h)
        .oneshot(multipart(&[("good.torrent", &good), ("copy.torrent", &good), ("bad.torrent", b"nope")]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = json(response).await;
    let statuses: Vec<&str> = body.as_array().unwrap().iter().map(|r| r["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["added", "duplicate", "invalid"]);
    assert_eq!(body[0]["file"], "good.torrent");
    assert!(body[0].get("message").is_none());
    assert!(!body[2]["message"].as_str().unwrap().is_empty());
    assert!(h.library.torrents_dir().join("good.torrent").exists());
    wait_for("queued torrent", || h.engine.snapshot().queued == 1).await;
}

#[tokio::test]
async fn config_roundtrip_and_validation() {
    let h = harness(test_config(), &[]).await;
    let current = json(app(&h).oneshot(get("/api/config")).await.unwrap()).await;
    assert_eq!(current["client"], "qbittorrent-5.2.4.client");

    let mut bad = current.clone();
    bad["maxUploadRate"] = 1.into();
    let response = app(&h).oneshot(put_json("/api/config", &bad)).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = json(response).await;
    assert_eq!(error["field"], "maxUploadRate");
    assert!(!error["message"].as_str().unwrap().is_empty());

    let mut good = current.clone();
    good["simultaneousSeed"] = 3.into();
    let response = app(&h).oneshot(put_json("/api/config", &good)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["simultaneousSeed"], 3);
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(h.dir.path().join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["simultaneousSeed"], 3);
}

#[tokio::test]
async fn lists_client_profiles_sorted() {
    let h = harness(test_config(), &[]).await;
    let body = json(app(&h).oneshot(get("/api/clients")).await.unwrap()).await;
    let names: Vec<&str> = body.as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(names.len(), 95);
    assert_eq!(names.iter().find(|n| n.starts_with("qbittorrent-")), Some(&"qbittorrent-5.2.4.client"));
}

#[tokio::test]
async fn delete_unknown_or_malformed_hash_is_404() {
    let h = harness(test_config(), &[]).await;
    for uri in ["/api/torrents/zz", &format!("/api/torrents/{}", "00".repeat(20))] {
        let response = app(&h).oneshot(Request::delete(uri).body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{uri}");
    }
}

#[tokio::test]
async fn start_and_stop_endpoints() {
    let h = harness(test_config(), &[]).await;
    let response = app(&h).oneshot(Request::post("/api/seed/start").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    wait_for("running", || h.engine.snapshot().running).await;
    let response = app(&h).oneshot(Request::post("/api/seed/stop").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(!h.engine.snapshot().running);
}

#[tokio::test]
async fn events_stream_starts_with_a_snapshot() {
    let h = harness(test_config(), &[]).await;
    let response = app(&h).oneshot(get("/api/events")).await.unwrap();
    assert_eq!(response.headers()[header::CONTENT_TYPE], "text/event-stream");
    let mut body = response.into_body();
    let frame = body.frame().await.unwrap().unwrap();
    let text = String::from_utf8(frame.into_data().unwrap().to_vec()).unwrap();
    assert!(text.starts_with("event: snapshot\ndata: {"), "{text}");
    assert!(text.contains("\"running\":false"), "{text}");
}

#[tokio::test]
async fn static_fallback() {
    let h = harness(test_config(), &[]).await;
    let missing_asset = app(&h).oneshot(get("/assets/missing.js")).await.unwrap();
    assert_eq!(missing_asset.status(), StatusCode::NOT_FOUND);
    let page = app(&h).oneshot(get("/")).await.unwrap();
    assert!(
        page.status() == StatusCode::OK || page.status() == StatusCode::SERVICE_UNAVAILABLE,
        "index is served when built, 503 otherwise: {}",
        page.status()
    );
}

#[tokio::test]
async fn desktop_show_calls_the_callback_in_desktop_mode() {
    let h = harness(test_config(), &[]).await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = calls.clone();
    let on_show: std::sync::Arc<dyn Fn() + Send + Sync> =
        std::sync::Arc::new(move || _ = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let response = app_with(&h, Some(on_show))
        .oneshot(Request::post("/api/desktop/show").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn desktop_show_is_404_outside_desktop_mode() {
    let h = harness(test_config(), &[]).await;
    let response = app(&h).oneshot(Request::post("/api/desktop/show").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
