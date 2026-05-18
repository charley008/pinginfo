use std::convert::Infallible;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, watch};
use tokio_stream::wrappers::BroadcastStream;
use tower_http::services::{ServeDir, ServeFile};

use crate::db;
use crate::models::{NewTarget, Target};
use crate::scheduler::{Scheduler, ServerEvent, SharedStatuses};

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub statuses: SharedStatuses,
    pub events: broadcast::Sender<ServerEvent>,
    pub scheduler: Scheduler,
    shutdown: watch::Receiver<bool>,
}

impl AppState {
    #[cfg(test)]
    pub fn with_statuses(
        pool: SqlitePool,
        statuses: SharedStatuses,
        scheduler: Scheduler,
        events: broadcast::Sender<ServerEvent>,
    ) -> Self {
        let (_, shutdown) = watch::channel(false);
        Self::with_shutdown(pool, statuses, scheduler, events, shutdown)
    }

    pub fn with_shutdown(
        pool: SqlitePool,
        statuses: SharedStatuses,
        scheduler: Scheduler,
        events: broadcast::Sender<ServerEvent>,
        shutdown: watch::Receiver<bool>,
    ) -> Self {
        Self {
            pool,
            statuses,
            events,
            scheduler,
            shutdown,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ResultsQuery {
    limit: Option<i64>,
    minutes: Option<i64>,
    before: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RollupQuery {
    minutes: Option<i64>,
    before: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BulkTargetsRequest {
    targets: Vec<NewTarget>,
}

#[derive(Debug, Serialize)]
pub struct BulkTargetsResponse {
    created: Vec<Target>,
    skipped: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MetaResponse {
    version: &'static str,
}

pub fn router(state: AppState) -> Router {
    let static_service =
        ServeDir::new("static").not_found_service(ServeFile::new("static/index.html"));

    Router::new()
        .route("/api/targets/bulk", post(create_targets_bulk))
        .route("/api/targets", get(list_targets).post(create_target))
        .route(
            "/api/targets/{id}",
            put(update_target).delete(delete_target),
        )
        .route("/api/targets/{id}/enable", post(enable_target))
        .route("/api/targets/{id}/disable", post(disable_target))
        .route("/api/targets/{id}/clear-data", post(clear_target_data))
        .route("/api/meta", get(meta))
        .route("/api/status", get(status))
        .route("/api/targets/{id}/results", get(recent_results))
        .route("/api/targets/{id}/rollup", get(rollups))
        .route("/api/targets/{id}/failure-events", get(failure_events))
        .route("/api/events", get(events))
        .fallback_service(static_service)
        .with_state(state)
}

async fn create_targets_bulk(
    State(state): State<AppState>,
    Json(input): Json<BulkTargetsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let existing = db::list_targets(&state.pool).await?;
    let mut known = existing
        .iter()
        .map(target_key)
        .collect::<std::collections::HashSet<_>>();
    let mut created = Vec::new();
    let mut skipped = Vec::new();

    for target in input.targets {
        let key = new_target_key(&target);
        if known.contains(&key) {
            skipped.push(format!(
                "{}{}",
                target.host,
                target.port.map(|port| format!(":{port}")).unwrap_or_default()
            ));
            continue;
        }
        let created_target = db::create_target(&state.pool, &target).await?;
        state.scheduler.start_or_replace(created_target.clone()).await;
        known.insert(key);
        created.push(created_target);
    }

    let _ = state.events.send(ServerEvent::TargetsChanged);
    Ok(Json(BulkTargetsResponse { created, skipped }))
}

async fn list_targets(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(db::list_targets(&state.pool).await?))
}

fn target_key(target: &Target) -> String {
    format!(
        "{}:{}:{}",
        target.probe_type.as_str(),
        target.host.to_lowercase(),
        target.port.map(|port| port.to_string()).unwrap_or_default()
    )
}

fn new_target_key(target: &NewTarget) -> String {
    format!(
        "{}:{}:{}",
        target.probe_type.as_str(),
        target.host.to_lowercase(),
        target.port.map(|port| port.to_string()).unwrap_or_default()
    )
}

async fn create_target(
    State(state): State<AppState>,
    Json(input): Json<NewTarget>,
) -> Result<impl IntoResponse, ApiError> {
    let target = db::create_target(&state.pool, &input).await?;
    state.scheduler.start_or_replace(target.clone()).await;
    let _ = state.events.send(ServerEvent::TargetsChanged);
    Ok((StatusCode::CREATED, Json(target)))
}

async fn update_target(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(input): Json<NewTarget>,
) -> Result<impl IntoResponse, ApiError> {
    let target = db::update_target(&state.pool, id, &input).await?;
    state.scheduler.start_or_replace(target.clone()).await;
    let _ = state.events.send(ServerEvent::TargetsChanged);
    Ok(Json(target))
}

async fn delete_target(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    state.scheduler.stop(id).await;
    db::delete_target(&state.pool, id).await?;
    state.statuses.write().await.remove(&id);
    let _ = state.events.send(ServerEvent::TargetsChanged);
    Ok(StatusCode::NO_CONTENT)
}

async fn enable_target(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let target = db::set_target_enabled(&state.pool, id, true).await?;
    state.scheduler.start_or_replace(target.clone()).await;
    let _ = state.events.send(ServerEvent::TargetsChanged);
    Ok(Json(target))
}

async fn disable_target(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let target = db::set_target_enabled(&state.pool, id, false).await?;
    state.scheduler.start_or_replace(target.clone()).await;
    let _ = state.events.send(ServerEvent::TargetsChanged);
    Ok(Json(target))
}

async fn clear_target_data(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<impl IntoResponse, ApiError> {
    let target = db::get_target(&state.pool, id).await?;
    db::clear_target_data(&state.pool, id).await?;

    let status = if target.enabled {
        crate::status::TargetStatus::new(id)
    } else {
        crate::status::TargetStatus::disabled(id)
    };
    {
        let mut statuses = state.statuses.write().await;
        statuses.insert(id, status.clone());
    }
    let _ = state.events.send(ServerEvent::Status { status: status.clone() });
    Ok(Json(status))
}

async fn meta() -> impl IntoResponse {
    Json(MetaResponse {
        version: crate::version::VERSION,
    })
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    let statuses: Vec<_> = state.statuses.read().await.values().cloned().collect();
    Json(statuses)
}

async fn recent_results(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<ResultsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if let Some(minutes) = query.minutes {
        let end_at = match query.before {
            Some(value) => DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc),
            None => Utc::now(),
        };
        return Ok(Json(
            db::results_window(&state.pool, id, minutes, end_at).await?,
        ));
    }

    Ok(Json(
        db::recent_results(&state.pool, id, query.limit.unwrap_or(500)).await?,
    ))
}

async fn rollups(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<RollupQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let end_at = match query.before {
        Some(value) => DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc),
        None => Utc::now(),
    };
    Ok(Json(
        db::rollups_1m_window(&state.pool, id, query.minutes.unwrap_or(60), end_at).await?,
    ))
}

async fn failure_events(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<RollupQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let end_at = match query.before {
        Some(value) => DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc),
        None => Utc::now(),
    };
    Ok(Json(
        db::failure_events_window(&state.pool, id, query.minutes.unwrap_or(60), end_at, 90)
            .await?,
    ))
}

async fn events(
    State(state): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let mut shutdown = state.shutdown.clone();
    let stream = BroadcastStream::new(state.events.subscribe()).filter_map(|message| async move {
        match message {
            Ok(event) => serde_json::to_string(&event)
                .ok()
                .map(|json| Ok(Event::default().data(json))),
            Err(_) => None,
        }
    });
    let stream = stream.take_until(async move {
        while shutdown.changed().await.is_ok() {
            if *shutdown.borrow() {
                break;
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Debug)]
pub struct ApiError(anyhow::Error);

impl<E> From<E> for ApiError
where
    E: Into<anyhow::Error>,
{
    fn from(value: E) -> Self {
        Self(value.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(serde_json::json!({
            "error": self.0.to_string(),
        }));
        (StatusCode::BAD_REQUEST, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use serde_json::json;
    use tokio::sync::{RwLock, broadcast, watch};
    use tower::ServiceExt;

    use crate::api::{AppState, router};
    use crate::db;
    use crate::models::{ProbeResult, ProbeType};
    use crate::scheduler::Scheduler;

    async fn test_app() -> (axum::Router, sqlx::SqlitePool) {
        let pool = db::connect_memory().await.unwrap();
        let statuses = Arc::new(RwLock::new(HashMap::new()));
        let (events, _) = broadcast::channel(16);
        let scheduler = Scheduler::new(pool.clone(), statuses.clone(), events.clone());
        let state = AppState::with_statuses(pool.clone(), statuses, scheduler, events);
        (router(state), pool)
    }

    #[tokio::test]
    async fn post_targets_creates_a_disabled_target() {
        let (app, _pool) = test_app().await;
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/targets")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "name": "Web",
                    "host": "127.0.0.1",
                    "probe_type": "tcp",
                    "port": 80,
                    "interval_ms": 1000,
                    "timeout_ms": 1000,
                    "enabled": false,
                    "group_name": null,
                    "description": null
                })
                .to_string(),
            ))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["name"], "Web");
        assert_eq!(payload["enabled"], false);
    }

    #[tokio::test]
    async fn meta_endpoint_returns_current_version() {
        let (app, _pool) = test_app().await;
        let request = Request::builder().uri("/api/meta").body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["version"], crate::version::VERSION);
    }

    #[tokio::test]
    async fn rollup_endpoint_returns_minute_buckets() {
        let (app, pool) = test_app().await;
        let target = db::create_target(
            &pool,
            &crate::models::NewTarget {
                name: "Web".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Tcp,
                port: Some(80),
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: false,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();
        let result = ProbeResult::success_for_test(target.id, 8.0);
        db::upsert_rollup_for_result(&pool, &result).await.unwrap();

        let request = Request::builder()
            .uri(format!("/api/targets/{}/rollup?minutes=60", target.id))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.as_array().unwrap().len(), 1);
        assert_eq!(payload[0]["success_count"], 1);
    }

    #[tokio::test]
    async fn results_endpoint_accepts_time_window_queries() {
        let (app, pool) = test_app().await;
        let target = db::create_target(
            &pool,
            &crate::models::NewTarget {
                name: "Web".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Tcp,
                port: Some(80),
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: false,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        let mut result = ProbeResult::failure_for_test(target.id, "timeout");
        result.started_at = chrono::Utc::now() - chrono::Duration::minutes(5);
        result.finished_at = result.started_at;
        db::insert_results(&pool, &[result.clone()]).await.unwrap();

        let before = (chrono::Utc::now() + chrono::Duration::minutes(1))
            .to_rfc3339()
            .replace(":", "%3A")
            .replace("+", "%2B");
        let request = Request::builder()
            .uri(format!(
                "/api/targets/{}/results?minutes=10&before={}",
                target.id, before
            ))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.as_array().unwrap().len(), 1);
        assert_eq!(payload[0]["success"], false);
    }

    #[tokio::test]
    async fn rollup_endpoint_accepts_before_window_end() {
        let (app, pool) = test_app().await;
        let target = db::create_target(
            &pool,
            &crate::models::NewTarget {
                name: "Web".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Tcp,
                port: Some(80),
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: false,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();
        let mut result = ProbeResult::success_for_test(target.id, 18.0);
        result.started_at = chrono::Utc::now() - chrono::Duration::hours(5);
        result.finished_at = result.started_at;
        db::upsert_rollup_for_result(&pool, &result).await.unwrap();

        let before = (result.finished_at + chrono::Duration::minutes(1))
            .to_rfc3339()
            .replace(":", "%3A")
            .replace("+", "%2B");
        let request = Request::builder()
            .uri(format!("/api/targets/{}/rollup?minutes=10&before={}", target.id, before))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.as_array().unwrap().len(), 1);
        assert_eq!(payload[0]["avg_latency_ms"], 18.0);
    }

    #[tokio::test]
    async fn failure_events_endpoint_returns_grouped_loss_ranges() {
        let (app, pool) = test_app().await;
        let target = db::create_target(
            &pool,
            &crate::models::NewTarget {
                name: "Web".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Tcp,
                port: Some(80),
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: false,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();

        let base = chrono::Utc::now() - chrono::Duration::minutes(5);
        let mut first = ProbeResult::failure_for_test(target.id, "timeout");
        first.started_at = base;
        first.finished_at = first.started_at;
        let mut second = ProbeResult::failure_for_test(target.id, "timeout");
        second.started_at = base + chrono::Duration::seconds(30);
        second.finished_at = second.started_at;
        db::insert_results(&pool, &[first, second]).await.unwrap();

        let request = Request::builder()
            .uri(format!("/api/targets/{}/failure-events?minutes=10", target.id))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.as_array().unwrap().len(), 1);
        assert_eq!(payload[0]["failure_count"], 2);
    }

    #[tokio::test]
    async fn events_stream_ends_when_shutdown_is_requested() {
        let pool = db::connect_memory().await.unwrap();
        let statuses = Arc::new(RwLock::new(HashMap::new()));
        let (events, _) = broadcast::channel(16);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let scheduler = Scheduler::new(pool.clone(), statuses.clone(), events.clone());
        let state = AppState::with_shutdown(pool, statuses, scheduler, events, shutdown_rx);
        let app = router(state);

        let request = Request::builder()
            .uri("/api/events")
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        shutdown_tx.send(true).unwrap();
        let body = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            to_bytes(response.into_body(), usize::MAX),
        )
        .await
        .expect("SSE stream should end after shutdown")
        .unwrap();
        assert!(body.is_empty());
    }

    #[tokio::test]
    async fn clear_data_endpoint_removes_selected_target_history() {
        let (app, pool) = test_app().await;
        let target = db::create_target(
            &pool,
            &crate::models::NewTarget {
                name: "Web".into(),
                host: "127.0.0.1".into(),
                probe_type: ProbeType::Tcp,
                port: Some(80),
                interval_ms: 1000,
                timeout_ms: 1000,
                enabled: true,
                group_name: None,
                description: None,
            },
        )
        .await
        .unwrap();
        let result = ProbeResult::success_for_test(target.id, 8.0);
        db::insert_results(&pool, std::slice::from_ref(&result))
            .await
            .unwrap();
        db::upsert_rollup_for_result(&pool, &result).await.unwrap();

        let request = Request::builder()
            .method(Method::POST)
            .uri(format!("/api/targets/{}/clear-data", target.id))
            .body(Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        assert!(db::recent_results(&pool, target.id, 10).await.unwrap().is_empty());
        assert!(db::rollups_1m_window(&pool, target.id, 60, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn bulk_targets_create_multiple_targets_at_once() {
        let (app, _pool) = test_app().await;
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/targets/bulk")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "targets": [
                        {
                            "name": "DNS",
                            "host": "223.5.5.5",
                            "probe_type": "icmp",
                            "port": null,
                            "interval_ms": 1000,
                            "timeout_ms": 1000,
                            "enabled": false,
                            "group_name": null,
                            "description": null
                        },
                        {
                            "name": "Web",
                            "host": "example.com",
                            "probe_type": "tcp",
                            "port": 443,
                            "interval_ms": 1000,
                            "timeout_ms": 1000,
                            "enabled": false,
                            "group_name": null,
                            "description": null
                        }
                    ]
                })
                .to_string(),
            ))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["created"].as_array().unwrap().len(), 2);
        assert_eq!(payload["skipped"].as_array().unwrap().len(), 0);
    }
}
