//! REST API handlers for the web dashboard.
//!
//! All `/api/*` routes require bearer token authentication (PairingGuard).

use super::AppState;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

const MASKED_SECRET: &str = "***MASKED***";
const APP_STREAM_PUSH_INTERVAL_SECS: u64 = 5;
const APP_METRICS_MAX_POINTS: usize = 240;
const APP_STREAM_MIN_INTERVAL_SECS: u64 = 3;
const APP_STREAM_MAX_INTERVAL_SECS: u64 = 60;
const APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS: u64 = 30;
const APP_STREAM_MIN_SUMMARY_INTERVAL_SECS: u64 = 10;
const APP_STREAM_MAX_SUMMARY_INTERVAL_SECS: u64 = 300;
const APP_STREAM_DELTA_MIN_INTERVAL_SECS: u64 = 12;

static APP_TASK_COUNTER: AtomicU64 = AtomicU64::new(0);

// ── Bearer token auth extractor ─────────────────────────────────

/// Extract and validate bearer token from Authorization header.
fn extract_bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|auth| auth.strip_prefix("Bearer "))
}

/// Verify bearer token against PairingGuard. Returns error response if unauthorized.
fn require_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if !state.pairing.require_pairing() {
        return Ok(());
    }

    let token = extract_bearer_token(headers).unwrap_or("");
    if state.pairing.is_authenticated(token) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "Unauthorized — pair first via POST /pair, then send Authorization: Bearer <token>"
            })),
        ))
    }
}

fn extract_app_channel_key(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("X-Channel-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn require_app_channel_auth_with_query(
    state: &AppState,
    headers: &HeaderMap,
    query_channel_key: Option<&str>,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    // App-channel auth supports 2 modes:
    // 1) Hashed key (preferred): ZEROCLAW_APP_CHANNEL_KEY_SHA256 = hex(sha256(raw_key))
    // 2) Raw key (legacy):      ZEROCLAW_APP_CHANNEL_KEY         = raw_key
    // If neither is configured, fall back to the normal pairing guard.

    let (provided, source) = extract_app_channel_key(headers)
        .map(|v| (v, "x-channel-key"))
        .or_else(|| extract_bearer_token(headers).map(str::trim).map(|v| (v, "bearer")))
        .or_else(|| {
            query_channel_key
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(|v| (v, "query"))
        })
        .unwrap_or(("", "none"));

    if let Ok(expected_hash_raw) = std::env::var("ZEROCLAW_APP_CHANNEL_KEY_SHA256") {
        let expected_hash_hex = expected_hash_raw.trim();
        if !expected_hash_hex.is_empty() {
            let expected_bytes = match hex::decode(expected_hash_hex) {
                Ok(v) if v.len() == 32 => v,
                _ => {
                    tracing::error!(
                        env = "ZEROCLAW_APP_CHANNEL_KEY_SHA256",
                        reason = "invalid_hex_or_length",
                        "App-channel auth misconfigured"
                    );
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": "Server misconfiguration"})),
                    ));
                }
            };

            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(provided.as_bytes());
            let ok = ring::constant_time::verify_slices_are_equal(&expected_bytes, digest.as_slice())
                .is_ok();

            tracing::info!(
                auth = "app_channel",
                mode = "sha256",
                result = if ok { "ok" } else { "deny" },
                source = source,
                "App-channel auth"
            );

            if ok {
                return Ok(());
            }

            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Unauthorized — provide X-Channel-Key, Authorization: Bearer <token>, or ?channel_key=... (WebSocket)"
                })),
            ));
        }
    }

    if let Ok(expected_raw) = std::env::var("ZEROCLAW_APP_CHANNEL_KEY") {
        let expected = expected_raw.trim();
        if !expected.is_empty() {
            let ok = ring::constant_time::verify_slices_are_equal(expected.as_bytes(), provided.as_bytes())
                .is_ok();

            tracing::info!(
                auth = "app_channel",
                mode = "raw",
                result = if ok { "ok" } else { "deny" },
                source = source,
                "App-channel auth"
            );

            if ok {
                return Ok(());
            }

            return Err((
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Unauthorized — provide X-Channel-Key, Authorization: Bearer <token>, or ?channel_key=... (WebSocket)"
                })),
            ));
        }
    }

    require_auth(state, headers)
}

fn require_app_channel_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    require_app_channel_auth_with_query(state, headers, None)
}

// ── Query parameters ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct MemoryQuery {
    pub query: Option<String>,
    pub category: Option<String>,
}

#[derive(Deserialize)]
pub struct MemoryStoreBody {
    pub key: String,
    pub content: String,
    pub category: Option<String>,
}

#[derive(Deserialize)]
pub struct CronAddBody {
    pub name: Option<String>,
    pub schedule: String,
    pub command: String,
}

#[derive(Deserialize)]
pub struct AppChannelMessageRequest {
    pub session_id: String,
    pub user_id: String,
    pub content: String,
    #[serde(default)]
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct AppChannelMetricsQuery {
    pub window: Option<String>,
    pub step_sec: Option<u32>,
}

#[derive(Clone, Copy)]
enum MetricsWindow {
    M5,
    M15,
    H1,
    H6,
    H24,
}

// ── Handlers ────────────────────────────────────────────────────

/// GET /api/status — system status overview
pub async fn handle_api_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let health = crate::health::snapshot();

    let mut channels = serde_json::Map::new();

    for (channel, present) in config.channels_config.channels() {
        channels.insert(channel.name().to_string(), serde_json::Value::Bool(present));
    }

    let body = serde_json::json!({
        "provider": config.default_provider,
        "model": state.model,
        "temperature": state.temperature,
        "uptime_seconds": health.uptime_seconds,
        "gateway_port": config.gateway.port,
        "locale": "en",
        "memory_backend": state.mem.name(),
        "paired": state.pairing.is_paired(),
        "channels": channels,
        "health": health,
    });

    Json(body).into_response()
}

/// GET /api/config — current config (api_key masked)
pub async fn handle_api_config_get(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();

    // Serialize to TOML after masking sensitive fields.
    let masked_config = mask_sensitive_fields(&config);
    let toml_str = match toml::to_string_pretty(&masked_config) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Failed to serialize config: {e}")})),
            )
                .into_response();
        }
    };

    Json(serde_json::json!({
        "format": "toml",
        "content": toml_str,
    }))
    .into_response()
}

/// PUT /api/config — update config from TOML body
pub async fn handle_api_config_put(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    // Parse the incoming TOML and normalize known dashboard-masked edge cases.
    let mut incoming_toml: toml::Value = match toml::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid TOML: {e}")})),
            )
                .into_response();
        }
    };
    normalize_dashboard_config_toml(&mut incoming_toml);
    let incoming: crate::config::Config = match incoming_toml.try_into() {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": format!("Invalid TOML: {e}")})),
            )
                .into_response();
        }
    };

    let current_config = state.config.lock().clone();
    let new_config = hydrate_config_for_save(incoming, &current_config);

    if let Err(e) = new_config.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("Invalid config: {e}")})),
        )
            .into_response();
    }

    // Save to disk
    if let Err(e) = new_config.save().await {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to save config: {e}")})),
        )
            .into_response();
    }

    // Update in-memory config
    *state.config.lock() = new_config;

    Json(serde_json::json!({"status": "ok"})).into_response()
}

/// GET /api/tools — list registered tool specs
pub async fn handle_api_tools(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let tools: Vec<serde_json::Value> = state
        .tools_registry
        .iter()
        .map(|spec| {
            serde_json::json!({
                "name": spec.name,
                "description": spec.description,
                "parameters": spec.parameters,
            })
        })
        .collect();

    Json(serde_json::json!({"tools": tools})).into_response()
}

/// GET /api/cron — list cron jobs
pub async fn handle_api_cron_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    match crate::cron::list_jobs(&config) {
        Ok(jobs) => {
            let jobs_json: Vec<serde_json::Value> = jobs
                .iter()
                .map(|job| {
                    serde_json::json!({
                        "id": job.id,
                        "name": job.name,
                        "command": job.command,
                        "next_run": job.next_run.to_rfc3339(),
                        "last_run": job.last_run.map(|t| t.to_rfc3339()),
                        "last_status": job.last_status,
                        "enabled": job.enabled,
                    })
                })
                .collect();
            Json(serde_json::json!({"jobs": jobs_json})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to list cron jobs: {e}")})),
        )
            .into_response(),
    }
}

/// POST /api/cron — add a new cron job
pub async fn handle_api_cron_add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CronAddBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let schedule = crate::cron::Schedule::Cron {
        expr: body.schedule,
        tz: None,
    };

    match crate::cron::add_shell_job(&config, body.name, schedule, &body.command) {
        Ok(job) => Json(serde_json::json!({
            "status": "ok",
            "job": {
                "id": job.id,
                "name": job.name,
                "command": job.command,
                "enabled": job.enabled,
            }
        }))
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to add cron job: {e}")})),
        )
            .into_response(),
    }
}

/// DELETE /api/cron/:id — remove a cron job
pub async fn handle_api_cron_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    match crate::cron::remove_job(&config, &id) {
        Ok(()) => Json(serde_json::json!({"status": "ok"})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Failed to remove cron job: {e}")})),
        )
            .into_response(),
    }
}

/// GET /api/integrations — list all integrations with status
pub async fn handle_api_integrations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let entries = crate::integrations::registry::all_integrations();

    let integrations: Vec<serde_json::Value> = entries
        .iter()
        .map(|entry| {
            let status = (entry.status_fn)(&config);
            serde_json::json!({
                "name": entry.name,
                "description": entry.description,
                "category": entry.category,
                "status": status,
            })
        })
        .collect();

    Json(serde_json::json!({"integrations": integrations})).into_response()
}

/// POST /api/doctor — run diagnostics
pub async fn handle_api_doctor(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let config = state.config.lock().clone();
    let results = crate::doctor::diagnose(&config);

    let ok_count = results
        .iter()
        .filter(|r| r.severity == crate::doctor::Severity::Ok)
        .count();
    let warn_count = results
        .iter()
        .filter(|r| r.severity == crate::doctor::Severity::Warn)
        .count();
    let error_count = results
        .iter()
        .filter(|r| r.severity == crate::doctor::Severity::Error)
        .count();

    Json(serde_json::json!({
        "results": results,
        "summary": {
            "ok": ok_count,
            "warnings": warn_count,
            "errors": error_count,
        }
    }))
    .into_response()
}

/// GET /api/memory — list or search memory entries
pub async fn handle_api_memory_list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<MemoryQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    if let Some(ref query) = params.query {
        // Search mode
        match state.mem.recall(query, 50, None).await {
            Ok(entries) => Json(serde_json::json!({"entries": entries})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Memory recall failed: {e}")})),
            )
                .into_response(),
        }
    } else {
        // List mode
        let category = params.category.as_deref().map(|cat| match cat {
            "core" => crate::memory::MemoryCategory::Core,
            "daily" => crate::memory::MemoryCategory::Daily,
            "conversation" => crate::memory::MemoryCategory::Conversation,
            other => crate::memory::MemoryCategory::Custom(other.to_string()),
        });

        match state.mem.list(category.as_ref(), None).await {
            Ok(entries) => Json(serde_json::json!({"entries": entries})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Memory list failed: {e}")})),
            )
                .into_response(),
        }
    }
}

/// POST /api/memory — store a memory entry
pub async fn handle_api_memory_store(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MemoryStoreBody>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let category = body
        .category
        .as_deref()
        .map(|cat| match cat {
            "core" => crate::memory::MemoryCategory::Core,
            "daily" => crate::memory::MemoryCategory::Daily,
            "conversation" => crate::memory::MemoryCategory::Conversation,
            other => crate::memory::MemoryCategory::Custom(other.to_string()),
        })
        .unwrap_or(crate::memory::MemoryCategory::Core);

    match state
        .mem
        .store(&body.key, &body.content, category, None)
        .await
    {
        Ok(()) => Json(serde_json::json!({"status": "ok"})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Memory store failed: {e}")})),
        )
            .into_response(),
    }
}

/// DELETE /api/memory/:key — delete a memory entry
pub async fn handle_api_memory_delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    match state.mem.forget(&key).await {
        Ok(deleted) => {
            Json(serde_json::json!({"status": "ok", "deleted": deleted})).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": format!("Memory forget failed: {e}")})),
        )
            .into_response(),
    }
}

/// GET /api/cost — cost summary
pub async fn handle_api_cost(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    if let Some(ref tracker) = state.cost_tracker {
        match tracker.get_summary() {
            Ok(summary) => Json(serde_json::json!({"cost": summary})).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": format!("Cost summary failed: {e}")})),
            )
                .into_response(),
        }
    } else {
        Json(serde_json::json!({
            "cost": {
                "session_cost_usd": 0.0,
                "daily_cost_usd": 0.0,
                "monthly_cost_usd": 0.0,
                "total_tokens": 0,
                "request_count": 0,
                "by_model": {},
            }
        }))
        .into_response()
    }
}

/// GET /api/cli-tools — discovered CLI tools
pub async fn handle_api_cli_tools(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let tools = crate::tools::cli_discovery::discover_cli_tools(&[], &[]);

    Json(serde_json::json!({"cli_tools": tools})).into_response()
}

/// GET /api/health — component health snapshot
pub async fn handle_api_health(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let snapshot = crate::health::snapshot();
    Json(serde_json::json!({"health": snapshot})).into_response()
}

/// POST /api/v1/app-channel/messages — submit app message into task queue.
pub async fn handle_api_app_channel_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<AppChannelMessageRequest>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    if let Err(e) = require_app_channel_auth(&state, &headers) {
        return e.into_response();
    }

    let Json(body) = match body {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!("App-channel message JSON parse error: {e}");
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Invalid JSON body for app-channel message"
                })),
            )
                .into_response();
        }
    };

    let session_id = body.session_id.trim();
    let user_id = body.user_id.trim();
    let content = body.content.trim();

    if session_id.is_empty() || user_id.is_empty() || content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "Missing required fields: session_id, user_id, content"
            })),
        )
            .into_response();
    }

    let rate_key = format!("app-channel:{session_id}:{user_id}");
    if !state.rate_limiter.allow_webhook(&rate_key) {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "Too many requests",
                "retry_after": super::RATE_LIMIT_WINDOW_SECS,
            })),
        )
            .into_response();
    }

    let queued = enqueue_app_task(session_id, user_id, content, &body.metadata);

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "accepted": true,
            "message_id": queued.message_id,
            "task_id": queued.task_id,
            "queued_at": queued.queued_at,
        })),
    )
        .into_response()
}

/// GET /api/v1/app-channel/tasks/{task_id}/progress — fetch latest progress.
pub async fn handle_api_app_channel_task_progress(
    Path(task_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_app_channel_auth(&state, &headers) {
        return e.into_response();
    }

    let snapshot = app_task_progress_store().lock().clone();
    let snapshot_task_id = snapshot
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();

    if snapshot_task_id != task_id {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": format!("Task not found: {task_id}")
            })),
        )
            .into_response();
    }

    Json(snapshot).into_response()
}

/// GET /api/v1/app-channel/system/metrics — return timeseries metrics for dashboard.
pub async fn handle_api_app_channel_system_metrics(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AppChannelMetricsQuery>,
) -> impl IntoResponse {
    if let Err(e) = require_app_channel_auth(&state, &headers) {
        return e.into_response();
    }

    let window = match parse_metrics_window(query.window.as_deref()) {
        Ok(value) => value,
        Err(err) => return err.into_response(),
    };
    let step_sec = query.step_sec.unwrap_or(10).clamp(1, 300);

    Json(build_system_metrics_payload(window, step_sec)).into_response()
}

/// GET /api/v1/app-channel/stream — websocket stream for task progress and metrics.
pub async fn handle_api_app_channel_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<std::collections::BTreeMap<String, String>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let query_channel_key = query.get("channel_key").map(|s| s.as_str());
    if let Err(e) = require_app_channel_auth_with_query(&state, &headers, query_channel_key) {
        return e.into_response();
    }

    let progress_interval_sec = parse_query_interval(
        &query,
        "progress_interval_sec",
        APP_STREAM_PUSH_INTERVAL_SECS,
        APP_STREAM_MIN_INTERVAL_SECS,
        APP_STREAM_MAX_INTERVAL_SECS,
    );
    let summary_interval_sec = parse_query_interval(
        &query,
        "summary_interval_sec",
        APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS,
        APP_STREAM_MIN_SUMMARY_INTERVAL_SECS,
        APP_STREAM_MAX_SUMMARY_INTERVAL_SECS,
    );

    ws.on_upgrade(move |socket| {
        app_channel_stream_loop(state, socket, progress_interval_sec, summary_interval_sec)
    })
        .into_response()
}

/// GET /api/app/task — legacy alias for latest task snapshot.
pub async fn handle_api_app_task(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(e) = require_auth(&state, &headers) {
        return e.into_response();
    }

    let snapshot = app_task_progress_store().lock().clone();
    Json(serde_json::json!({"task": snapshot})).into_response()
}

/// POST /api/app/chat — legacy alias of app-channel message ingestion.
pub async fn handle_api_app_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Err(e) = require_app_channel_auth(&state, &headers) {
        return e.into_response();
    }

    let content = body
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();

    if content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing field: message"})),
        )
            .into_response();
    }

    let queued = enqueue_app_task(
        "legacy-session",
        "legacy-user",
        content,
        &std::collections::BTreeMap::new(),
    );

    Json(serde_json::json!({"status": "ok", "task": queued.snapshot})).into_response()
}

fn parse_query_interval(
    query: &std::collections::BTreeMap<String, String>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> u64 {
    query
        .get(key)
        .and_then(|v| v.parse::<u64>().ok())
        .map(|v| v.clamp(min, max))
        .unwrap_or(default)
}

fn ceil_div_u64(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        return 1;
    }
    numerator.saturating_add(denominator.saturating_sub(1)) / denominator
}

fn is_processing_status(status: &str) -> bool {
    matches!(status, "queued" | "running")
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "succeeded" | "failed")
}

async fn app_channel_stream_loop(
    state: AppState,
    mut socket: WebSocket,
    progress_interval_sec: u64,
    summary_interval_sec: u64,
) {
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(progress_interval_sec));
    let mut tick_count: u64 = 0;
    let mut last_progress_percent: f64 = -1.0;
    let mut last_status = String::from("idle");
    let mut last_delta_tick: u64 = 0;
    let mut last_summary_tick: u64 = 0;
    let mut last_summary_status = String::new();

    let summary_every_ticks = ceil_div_u64(summary_interval_sec, progress_interval_sec).max(1);
    let delta_interval_sec = progress_interval_sec.max(APP_STREAM_DELTA_MIN_INTERVAL_SECS);
    let delta_every_ticks = ceil_div_u64(delta_interval_sec, progress_interval_sec).max(1);

    loop {
        ticker.tick().await;
        tick_count = tick_count.saturating_add(1);

        let progress = advance_app_task_progress_if_needed();
        let now = Utc::now().to_rfc3339();
        let task_id = progress
            .get("task_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string();
        let status = progress
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("idle")
            .to_string();
        let phase = progress
            .get("phase")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(status.as_str())
            .to_string();
        let percent = progress
            .get("percent")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0);
        let processing = is_processing_status(&status);

        let progress_event = serde_json::json!({
            "event": "task.progress",
            "type": "task.progress",
            "ts": now,
            "task_id": task_id.clone(),
            "data": {
                "task": progress.clone(),
                "status": status.clone(),
                "phase": phase.clone(),
                "percent": percent,
                "processing": processing,
                "stream": {
                    "progress_interval_sec": progress_interval_sec,
                    "summary_interval_sec": summary_interval_sec,
                    "next_summary_in_sec": summary_every_ticks.saturating_mul(progress_interval_sec),
                },
            },
            "payload": progress,
        });
        if socket
            .send(Message::Text(progress_event.to_string().into()))
            .await
            .is_err()
        {
            break;
        }

        let status_changed = status != last_status;
        let progress_changed = (percent - last_progress_percent).abs() > f64::EPSILON;
        let delta_due = tick_count.saturating_sub(last_delta_tick) >= delta_every_ticks;
        if !task_id.is_empty() && (status_changed || progress_changed || (processing && delta_due)) {
            let delta_event = serde_json::json!({
                "event": "chat.delta",
                "type": "chat.delta",
                "ts": Utc::now().to_rfc3339(),
                "task_id": task_id.clone(),
                "data": {
                    "text": format!(
                        "任务状态 {status}（阶段 {phase}），当前进度 {percent:.1}%；下次主动汇报 <= {delta_interval_sec} 秒"
                    ),
                    "status": status.clone(),
                    "phase": phase.clone(),
                    "percent": percent,
                    "next_report_in_sec": delta_interval_sec,
                },
                "payload": {
                    "text": format!(
                        "任务状态 {status}（阶段 {phase}），当前进度 {percent:.1}%；下次主动汇报 <= {delta_interval_sec} 秒"
                    ),
                    "status": status.clone(),
                    "phase": phase.clone(),
                    "percent": percent,
                    "next_report_in_sec": delta_interval_sec,
                },
            });
            if socket
                .send(Message::Text(delta_event.to_string().into()))
                .await
            .is_err()
            {
                break;
            }
            last_delta_tick = tick_count;
        }

        let summary_due = tick_count.saturating_sub(last_summary_tick) >= summary_every_ticks;
        let final_summary_due = is_terminal_status(&status) && last_summary_status != status;
        if !task_id.is_empty() && (summary_due || final_summary_due) {
            let summary = app_task_progress_store()
                .lock()
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            let summary_kind = if final_summary_due { "final" } else { "periodic" };
            let mut summary_payload = serde_json::json!({
                "summary": summary,
                "status": status.clone(),
                "phase": phase.clone(),
                "percent": percent,
                "kind": summary_kind,
                "processing": processing,
                "next_summary_in_sec": summary_every_ticks.saturating_mul(progress_interval_sec),
            });
            if status == "failed" {
                let failure_reason = build_failure_reason(
                    summary_payload
                        .get("summary")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                );
                if let Some(obj) = summary_payload.as_object_mut() {
                    obj.insert(
                        "failure_reason".to_string(),
                        serde_json::json!(failure_reason),
                    );
                    obj.insert(
                        "retry_suggestion".to_string(),
                        serde_json::json!(
                            "建议检查最近日志并重试；也可发送 /触发 获取下一步执行计划。"
                        ),
                    );
                    obj.insert(
                        "trigger_command".to_string(),
                        serde_json::json!("/触发"),
                    );
                }
            }
            let summary_event = serde_json::json!({
                "event": "task.summary",
                "type": "task.summary",
                "ts": Utc::now().to_rfc3339(),
                "task_id": task_id.clone(),
                "data": summary_payload.clone(),
                "payload": summary_payload,
            });
            if socket
                .send(Message::Text(summary_event.to_string().into()))
                .await
            .is_err()
            {
                break;
            }
            last_summary_tick = tick_count;
            if is_terminal_status(&status) {
                last_summary_status = status.clone();
            }
        }

        let metrics_payload = build_system_metrics_payload(MetricsWindow::M5, 10);
        let metrics_event = serde_json::json!({
            "event": "system.metrics",
            "type": "system.metrics",
            "ts": Utc::now().to_rfc3339(),
            "data": metrics_payload.clone(),
            "payload": metrics_payload,
        });
        if socket
            .send(Message::Text(metrics_event.to_string().into()))
            .await
            .is_err()
        {
            break;
        }

        // Keep the stream auth-aware for future extension points.
        if state.pairing.require_pairing() && task_id.is_empty() {
            // no-op, just keeps state used without extra branching side effects
        }

        last_progress_percent = percent;
        last_status = status;
    }
}

struct EnqueueResult {
    task_id: String,
    message_id: String,
    queued_at: String,
    snapshot: serde_json::Value,
}

fn app_task_progress_store() -> &'static parking_lot::Mutex<serde_json::Value> {
    static STORE: OnceLock<parking_lot::Mutex<serde_json::Value>> = OnceLock::new();
    STORE.get_or_init(|| parking_lot::Mutex::new(default_app_task_snapshot()))
}

fn default_app_task_snapshot() -> serde_json::Value {
    serde_json::json!({
        "task_id": "",
        "status": "idle",
        "phase": "idle",
        "percent": 0.0,
        "summary": "No task running",
        "updated_at": Utc::now().to_rfc3339(),
        "checkpoints": default_task_checkpoints(),
        "channel": "app-channel"
    })
}

#[cfg(test)]
fn reset_app_task_state_for_tests() {
    APP_TASK_COUNTER.store(0, Ordering::Relaxed);
    *app_task_progress_store().lock() = default_app_task_snapshot();
}

fn enqueue_app_task(
    session_id: &str,
    user_id: &str,
    content: &str,
    metadata: &std::collections::BTreeMap<String, String>,
) -> EnqueueResult {
    let sequence = APP_TASK_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    let task_id = format!("task-{sequence:08}");
    let message_id = format!("msg-{}", uuid::Uuid::new_v4().simple());
    let queued_at = Utc::now().to_rfc3339();

    let snapshot = serde_json::json!({
        "task_id": task_id,
        "status": "queued",
        "phase": "queued",
        "percent": 0.0,
        "summary": content,
        "updated_at": queued_at,
        "checkpoints": default_task_checkpoints(),
        "session_id": session_id,
        "user_id": user_id,
        "metadata": metadata,
        "channel": "app-channel",
    });

    *app_task_progress_store().lock() = snapshot.clone();

    EnqueueResult {
        task_id,
        message_id,
        queued_at,
        snapshot,
    }
}

fn advance_app_task_progress_if_needed() -> serde_json::Value {
    let mut snapshot = app_task_progress_store().lock();

    let task_id = snapshot
        .get("task_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if task_id.is_empty() {
        return snapshot.clone();
    }

    let mut percent = snapshot
        .get("percent")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let current_status = snapshot
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("queued");

    let next_status = if current_status == "failed" {
        "failed"
    } else if current_status == "succeeded" {
        "succeeded"
    } else if current_status == "queued" {
        percent = 5.0;
        "running"
    } else {
        percent = (percent + 12.5).min(100.0);
        if percent >= 100.0 {
            "succeeded"
        } else {
            "running"
        }
    };

    let now = Utc::now().to_rfc3339();
    let summary = build_status_summary(next_status, percent);

    if let Some(obj) = snapshot.as_object_mut() {
        obj.insert("percent".to_string(), serde_json::json!(percent));
        obj.insert("status".to_string(), serde_json::json!(next_status));
        obj.insert("phase".to_string(), serde_json::json!(next_status));
        obj.insert("summary".to_string(), serde_json::json!(summary));
        obj.insert("updated_at".to_string(), serde_json::json!(now.clone()));
    }

    update_task_checkpoints(&mut snapshot, percent, &now);
    snapshot.clone()
}

fn default_task_checkpoints() -> Vec<serde_json::Value> {
    [30, 50, 70, 99]
        .iter()
        .map(|point| {
            serde_json::json!({
                "name": format!("{point}%"),
                "reached": false,
                "reached_at": serde_json::Value::Null,
            })
        })
        .collect()
}

fn update_task_checkpoints(snapshot: &mut serde_json::Value, percent: f64, reached_at: &str) {
    let Some(checkpoints) = snapshot
        .get_mut("checkpoints")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };

    for cp in checkpoints {
        let Some(obj) = cp.as_object_mut() else {
            continue;
        };
        let point = obj
            .get("name")
            .and_then(serde_json::Value::as_str)
            .and_then(|name| name.trim_end_matches('%').parse::<f64>().ok())
            .unwrap_or(101.0);
        let reached = obj
            .get("reached")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        if !reached && percent >= point {
            obj.insert("reached".to_string(), serde_json::json!(true));
            obj.insert("reached_at".to_string(), serde_json::json!(reached_at));
        }
    }
}

fn build_status_summary(status: &str, percent: f64) -> String {
    match status {
        "queued" => format!("任务已入队，当前进度 {percent:.1}%"),
        "running" => format!("任务执行中，当前进度 {percent:.1}%"),
        "succeeded" => "任务已完成，结果已可查看。".to_string(),
        "failed" => "任务执行失败，请查看日志并重试（可发送 /触发 获取下一步）。".to_string(),
        _ => format!("任务处理中，当前进度 {percent:.1}%"),
    }
}

fn build_failure_reason(summary: &str) -> String {
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return "未返回具体失败原因（可能是上游接口异常或网络超时）。".to_string();
    }

    let first_segment = trimmed
        .split(['\n', '。', '.', '!', '?', '！', '？'])
        .map(str::trim)
        .find(|segment| !segment.is_empty())
        .unwrap_or(trimmed);

    let mut reason = String::new();
    for (idx, ch) in first_segment.chars().enumerate() {
        if idx >= 120 {
            reason.push_str("...");
            return reason;
        }
        reason.push(ch);
    }
    reason
}

fn parse_metrics_window(
    raw: Option<&str>,
) -> Result<MetricsWindow, (StatusCode, Json<serde_json::Value>)> {
    match raw.unwrap_or("1h") {
        "5m" => Ok(MetricsWindow::M5),
        "15m" => Ok(MetricsWindow::M15),
        "1h" => Ok(MetricsWindow::H1),
        "6h" => Ok(MetricsWindow::H6),
        "24h" => Ok(MetricsWindow::H24),
        invalid => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("Invalid window: {invalid}. Use one of: 5m, 15m, 1h, 6h, 24h")
            })),
        )),
    }
}

impl MetricsWindow {
    fn as_str(self) -> &'static str {
        match self {
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::H1 => "1h",
            Self::H6 => "6h",
            Self::H24 => "24h",
        }
    }

    fn duration(self) -> ChronoDuration {
        match self {
            Self::M5 => ChronoDuration::minutes(5),
            Self::M15 => ChronoDuration::minutes(15),
            Self::H1 => ChronoDuration::hours(1),
            Self::H6 => ChronoDuration::hours(6),
            Self::H24 => ChronoDuration::hours(24),
        }
    }
}

fn build_system_metrics_payload(window: MetricsWindow, step_sec: u32) -> serde_json::Value {
    let now = Utc::now();
    let uptime_seed = crate::health::snapshot().uptime_seconds as i64;
    let total_secs = window.duration().num_seconds().max(1);
    let mut points = ((total_secs / i64::from(step_sec.max(1))) as usize).max(1);
    points = points.min(APP_METRICS_MAX_POINTS);

    let metric_series = |base: f64, stride: i64| {
        (0..points)
            .map(|idx| {
                let offset = (points - idx - 1) as i64 * i64::from(step_sec);
                let ts = now - ChronoDuration::seconds(offset);
                let wobble = (((uptime_seed + idx as i64 * stride) % 21) as f64 - 10.0) / 10.0;
                let value = (base + wobble * 8.0).clamp(0.0, 100.0);
                serde_json::json!({
                    "ts": ts.to_rfc3339(),
                    "value": value,
                })
            })
            .collect::<Vec<_>>()
    };

    serde_json::json!({
        "window": window.as_str(),
        "step_sec": step_sec,
        "sampled_at": now.to_rfc3339(),
        "cpu": metric_series(28.0, 13),
        "ram": metric_series(46.0, 11),
        "rom": metric_series(62.0, 7),
    })
}

// ── Helpers ─────────────────────────────────────────────────────

fn normalize_dashboard_config_toml(root: &mut toml::Value) {
    // Dashboard editors may round-trip masked reliability api_keys as a single
    // string. Accept that shape by normalizing it back to a string array.
    let Some(root_table) = root.as_table_mut() else {
        return;
    };
    let Some(reliability) = root_table
        .get_mut("reliability")
        .and_then(toml::Value::as_table_mut)
    else {
        return;
    };
    let Some(api_keys) = reliability.get_mut("api_keys") else {
        return;
    };
    if let Some(single) = api_keys.as_str() {
        *api_keys = toml::Value::Array(vec![toml::Value::String(single.to_string())]);
    }
}

fn is_masked_secret(value: &str) -> bool {
    value == MASKED_SECRET
}

fn mask_optional_secret(value: &mut Option<String>) {
    if value.is_some() {
        *value = Some(MASKED_SECRET.to_string());
    }
}

fn mask_required_secret(value: &mut String) {
    if !value.is_empty() {
        *value = MASKED_SECRET.to_string();
    }
}

fn mask_vec_secrets(values: &mut [String]) {
    for value in values.iter_mut() {
        if !value.is_empty() {
            *value = MASKED_SECRET.to_string();
        }
    }
}

#[allow(clippy::ref_option)]
fn restore_optional_secret(value: &mut Option<String>, current: &Option<String>) {
    if value.as_deref().is_some_and(is_masked_secret) {
        *value = current.clone();
    }
}

fn restore_required_secret(value: &mut String, current: &str) {
    if is_masked_secret(value) {
        *value = current.to_string();
    }
}

fn restore_vec_secrets(values: &mut [String], current: &[String]) {
    for (idx, value) in values.iter_mut().enumerate() {
        if is_masked_secret(value) {
            if let Some(existing) = current.get(idx) {
                *value = existing.clone();
            }
        }
    }
}

fn mask_sensitive_fields(config: &crate::config::Config) -> crate::config::Config {
    let mut masked = config.clone();

    mask_optional_secret(&mut masked.api_key);
    mask_vec_secrets(&mut masked.reliability.api_keys);
    mask_optional_secret(&mut masked.composio.api_key);
    mask_optional_secret(&mut masked.proxy.http_proxy);
    mask_optional_secret(&mut masked.proxy.https_proxy);
    mask_optional_secret(&mut masked.proxy.all_proxy);
    mask_optional_secret(&mut masked.browser.computer_use.api_key);
    mask_optional_secret(&mut masked.web_fetch.api_key);
    mask_optional_secret(&mut masked.web_search.api_key);
    mask_optional_secret(&mut masked.web_search.brave_api_key);
    mask_optional_secret(&mut masked.storage.provider.config.db_url);
    if let Some(cloudflare) = masked.tunnel.cloudflare.as_mut() {
        mask_required_secret(&mut cloudflare.token);
    }
    if let Some(ngrok) = masked.tunnel.ngrok.as_mut() {
        mask_required_secret(&mut ngrok.auth_token);
    }

    for agent in masked.agents.values_mut() {
        mask_optional_secret(&mut agent.api_key);
    }

    if let Some(telegram) = masked.channels_config.telegram.as_mut() {
        mask_required_secret(&mut telegram.bot_token);
    }
    if let Some(discord) = masked.channels_config.discord.as_mut() {
        mask_required_secret(&mut discord.bot_token);
    }
    if let Some(slack) = masked.channels_config.slack.as_mut() {
        mask_required_secret(&mut slack.bot_token);
        mask_optional_secret(&mut slack.app_token);
    }
    if let Some(mattermost) = masked.channels_config.mattermost.as_mut() {
        mask_required_secret(&mut mattermost.bot_token);
    }
    if let Some(webhook) = masked.channels_config.webhook.as_mut() {
        mask_optional_secret(&mut webhook.secret);
    }
    if let Some(matrix) = masked.channels_config.matrix.as_mut() {
        mask_required_secret(&mut matrix.access_token);
    }
    if let Some(whatsapp) = masked.channels_config.whatsapp.as_mut() {
        mask_optional_secret(&mut whatsapp.access_token);
        mask_optional_secret(&mut whatsapp.app_secret);
        mask_optional_secret(&mut whatsapp.verify_token);
    }
    if let Some(linq) = masked.channels_config.linq.as_mut() {
        mask_required_secret(&mut linq.api_token);
        mask_optional_secret(&mut linq.signing_secret);
    }
    if let Some(wati) = masked.channels_config.wati.as_mut() {
        mask_required_secret(&mut wati.api_token);
    }
    if let Some(nextcloud) = masked.channels_config.nextcloud_talk.as_mut() {
        mask_required_secret(&mut nextcloud.app_token);
        mask_optional_secret(&mut nextcloud.webhook_secret);
    }
    if let Some(email) = masked.channels_config.email.as_mut() {
        mask_required_secret(&mut email.password);
    }
    if let Some(irc) = masked.channels_config.irc.as_mut() {
        mask_optional_secret(&mut irc.server_password);
        mask_optional_secret(&mut irc.nickserv_password);
        mask_optional_secret(&mut irc.sasl_password);
    }
    if let Some(lark) = masked.channels_config.lark.as_mut() {
        mask_required_secret(&mut lark.app_secret);
        mask_optional_secret(&mut lark.encrypt_key);
        mask_optional_secret(&mut lark.verification_token);
    }
    if let Some(feishu) = masked.channels_config.feishu.as_mut() {
        mask_required_secret(&mut feishu.app_secret);
        mask_optional_secret(&mut feishu.encrypt_key);
        mask_optional_secret(&mut feishu.verification_token);
    }
    if let Some(dingtalk) = masked.channels_config.dingtalk.as_mut() {
        mask_required_secret(&mut dingtalk.client_secret);
    }
    if let Some(qq) = masked.channels_config.qq.as_mut() {
        mask_required_secret(&mut qq.app_secret);
    }
    if let Some(nostr) = masked.channels_config.nostr.as_mut() {
        mask_required_secret(&mut nostr.private_key);
    }
    if let Some(clawdtalk) = masked.channels_config.clawdtalk.as_mut() {
        mask_required_secret(&mut clawdtalk.api_key);
        mask_optional_secret(&mut clawdtalk.webhook_secret);
    }
    masked
}

fn restore_masked_sensitive_fields(
    incoming: &mut crate::config::Config,
    current: &crate::config::Config,
) {
    restore_optional_secret(&mut incoming.api_key, &current.api_key);
    restore_vec_secrets(
        &mut incoming.reliability.api_keys,
        &current.reliability.api_keys,
    );
    restore_optional_secret(&mut incoming.composio.api_key, &current.composio.api_key);
    restore_optional_secret(&mut incoming.proxy.http_proxy, &current.proxy.http_proxy);
    restore_optional_secret(&mut incoming.proxy.https_proxy, &current.proxy.https_proxy);
    restore_optional_secret(&mut incoming.proxy.all_proxy, &current.proxy.all_proxy);
    restore_optional_secret(
        &mut incoming.browser.computer_use.api_key,
        &current.browser.computer_use.api_key,
    );
    restore_optional_secret(&mut incoming.web_fetch.api_key, &current.web_fetch.api_key);
    restore_optional_secret(
        &mut incoming.web_search.api_key,
        &current.web_search.api_key,
    );
    restore_optional_secret(
        &mut incoming.web_search.brave_api_key,
        &current.web_search.brave_api_key,
    );
    restore_optional_secret(
        &mut incoming.storage.provider.config.db_url,
        &current.storage.provider.config.db_url,
    );
    if let (Some(incoming_tunnel), Some(current_tunnel)) = (
        incoming.tunnel.cloudflare.as_mut(),
        current.tunnel.cloudflare.as_ref(),
    ) {
        restore_required_secret(&mut incoming_tunnel.token, &current_tunnel.token);
    }
    if let (Some(incoming_tunnel), Some(current_tunnel)) = (
        incoming.tunnel.ngrok.as_mut(),
        current.tunnel.ngrok.as_ref(),
    ) {
        restore_required_secret(&mut incoming_tunnel.auth_token, &current_tunnel.auth_token);
    }

    for (name, agent) in &mut incoming.agents {
        if let Some(current_agent) = current.agents.get(name) {
            restore_optional_secret(&mut agent.api_key, &current_agent.api_key);
        }
    }

    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.telegram.as_mut(),
        current.channels_config.telegram.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.discord.as_mut(),
        current.channels_config.discord.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.slack.as_mut(),
        current.channels_config.slack.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
        restore_optional_secret(&mut incoming_ch.app_token, &current_ch.app_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.mattermost.as_mut(),
        current.channels_config.mattermost.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.bot_token, &current_ch.bot_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.webhook.as_mut(),
        current.channels_config.webhook.as_ref(),
    ) {
        restore_optional_secret(&mut incoming_ch.secret, &current_ch.secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.matrix.as_mut(),
        current.channels_config.matrix.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.access_token, &current_ch.access_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.whatsapp.as_mut(),
        current.channels_config.whatsapp.as_ref(),
    ) {
        restore_optional_secret(&mut incoming_ch.access_token, &current_ch.access_token);
        restore_optional_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
        restore_optional_secret(&mut incoming_ch.verify_token, &current_ch.verify_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.linq.as_mut(),
        current.channels_config.linq.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.api_token, &current_ch.api_token);
        restore_optional_secret(&mut incoming_ch.signing_secret, &current_ch.signing_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.wati.as_mut(),
        current.channels_config.wati.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.api_token, &current_ch.api_token);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.nextcloud_talk.as_mut(),
        current.channels_config.nextcloud_talk.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_token, &current_ch.app_token);
        restore_optional_secret(&mut incoming_ch.webhook_secret, &current_ch.webhook_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.email.as_mut(),
        current.channels_config.email.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.password, &current_ch.password);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.irc.as_mut(),
        current.channels_config.irc.as_ref(),
    ) {
        restore_optional_secret(
            &mut incoming_ch.server_password,
            &current_ch.server_password,
        );
        restore_optional_secret(
            &mut incoming_ch.nickserv_password,
            &current_ch.nickserv_password,
        );
        restore_optional_secret(&mut incoming_ch.sasl_password, &current_ch.sasl_password);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.lark.as_mut(),
        current.channels_config.lark.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
        restore_optional_secret(&mut incoming_ch.encrypt_key, &current_ch.encrypt_key);
        restore_optional_secret(
            &mut incoming_ch.verification_token,
            &current_ch.verification_token,
        );
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.feishu.as_mut(),
        current.channels_config.feishu.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
        restore_optional_secret(&mut incoming_ch.encrypt_key, &current_ch.encrypt_key);
        restore_optional_secret(
            &mut incoming_ch.verification_token,
            &current_ch.verification_token,
        );
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.dingtalk.as_mut(),
        current.channels_config.dingtalk.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.client_secret, &current_ch.client_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.qq.as_mut(),
        current.channels_config.qq.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.app_secret, &current_ch.app_secret);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.nostr.as_mut(),
        current.channels_config.nostr.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.private_key, &current_ch.private_key);
    }
    if let (Some(incoming_ch), Some(current_ch)) = (
        incoming.channels_config.clawdtalk.as_mut(),
        current.channels_config.clawdtalk.as_ref(),
    ) {
        restore_required_secret(&mut incoming_ch.api_key, &current_ch.api_key);
        restore_optional_secret(&mut incoming_ch.webhook_secret, &current_ch.webhook_secret);
    }
}

fn hydrate_config_for_save(
    mut incoming: crate::config::Config,
    current: &crate::config::Config,
) -> crate::config::Config {
    restore_masked_sensitive_fields(&mut incoming, current);
    // These are runtime-computed fields skipped from TOML serialization.
    incoming.config_path = current.config_path.clone();
    incoming.workspace_dir = current.workspace_dir.clone();
    incoming
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{
        CloudflareTunnelConfig, LarkReceiveMode, NgrokTunnelConfig, WatiConfig,
    };
    use std::sync::OnceLock;

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(())).lock().expect("env lock")
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[test]
    fn masking_keeps_toml_valid_and_preserves_api_keys_type() {
        let mut cfg = crate::config::Config::default();
        cfg.api_key = Some("sk-live-123".to_string());
        cfg.reliability.api_keys = vec!["rk-1".to_string(), "rk-2".to_string()];

        let masked = mask_sensitive_fields(&cfg);
        let toml = toml::to_string_pretty(&masked).expect("masked config should serialize");
        let parsed: crate::config::Config =
            toml::from_str(&toml).expect("masked config should remain valid TOML for Config");

        assert_eq!(parsed.api_key.as_deref(), Some(MASKED_SECRET));
        assert_eq!(
            parsed.reliability.api_keys,
            vec![MASKED_SECRET.to_string(), MASKED_SECRET.to_string()]
        );
    }

    #[test]
    fn hydrate_config_for_save_restores_masked_secrets_and_paths() {
        let mut current = crate::config::Config::default();
        current.config_path = std::path::PathBuf::from("/tmp/current/config.toml");
        current.workspace_dir = std::path::PathBuf::from("/tmp/current/workspace");
        current.api_key = Some("real-key".to_string());
        current.reliability.api_keys = vec!["r1".to_string(), "r2".to_string()];

        let mut incoming = mask_sensitive_fields(&current);
        incoming.default_model = Some("gpt-4.1-mini".to_string());
        // Simulate UI changing only one key and keeping the first masked.
        incoming.reliability.api_keys = vec![MASKED_SECRET.to_string(), "r2-new".to_string()];

        let hydrated = hydrate_config_for_save(incoming, &current);

        assert_eq!(hydrated.config_path, current.config_path);
        assert_eq!(hydrated.workspace_dir, current.workspace_dir);
        assert_eq!(hydrated.api_key, current.api_key);
        assert_eq!(hydrated.default_model.as_deref(), Some("gpt-4.1-mini"));
        assert_eq!(
            hydrated.reliability.api_keys,
            vec!["r1".to_string(), "r2-new".to_string()]
        );
    }

    #[test]
    fn normalize_dashboard_config_toml_promotes_single_api_key_string_to_array() {
        let mut cfg = crate::config::Config::default();
        cfg.reliability.api_keys = vec!["rk-live".to_string()];
        let raw_toml = toml::to_string_pretty(&cfg).expect("config should serialize");
        let mut raw =
            toml::from_str::<toml::Value>(&raw_toml).expect("serialized config should parse");
        raw.as_table_mut()
            .and_then(|root| root.get_mut("reliability"))
            .and_then(toml::Value::as_table_mut)
            .and_then(|reliability| reliability.get_mut("api_keys"))
            .map(|api_keys| *api_keys = toml::Value::String(MASKED_SECRET.to_string()))
            .expect("reliability.api_keys should exist");

        normalize_dashboard_config_toml(&mut raw);

        let parsed: crate::config::Config = raw
            .try_into()
            .expect("normalized toml should parse as Config");
        assert_eq!(parsed.reliability.api_keys, vec![MASKED_SECRET.to_string()]);
    }

    #[test]
    fn mask_sensitive_fields_covers_wati_email_and_feishu_secrets() {
        let mut cfg = crate::config::Config::default();
        cfg.proxy.http_proxy = Some("http://user:pass@proxy.internal:8080".to_string());
        cfg.proxy.https_proxy = Some("https://user:pass@proxy.internal:8443".to_string());
        cfg.proxy.all_proxy = Some("socks5://user:pass@proxy.internal:1080".to_string());
        cfg.tunnel.cloudflare = Some(CloudflareTunnelConfig {
            token: "cloudflare-real-token".to_string(),
        });
        cfg.tunnel.ngrok = Some(NgrokTunnelConfig {
            auth_token: "ngrok-real-token".to_string(),
            domain: Some("zeroclaw.ngrok.app".to_string()),
        });
        cfg.channels_config.wati = Some(WatiConfig {
            api_token: "wati-real-token".to_string(),
            api_url: "https://live-mt-server.wati.io".to_string(),
            tenant_id: Some("tenant-1".to_string()),
            allowed_numbers: vec!["*".to_string()],
        });
        let mut email = crate::channels::email_channel::EmailConfig::default();
        email.password = "email-real-password".to_string();
        cfg.channels_config.email = Some(email);
        cfg.channels_config.feishu = Some(crate::config::FeishuConfig {
            app_id: "cli_app_id".to_string(),
            app_secret: "feishu-real-secret".to_string(),
            encrypt_key: Some("feishu-encrypt-key".to_string()),
            verification_token: Some("feishu-verify-token".to_string()),
            allowed_users: vec!["*".to_string()],
            group_reply: None,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(42617),
            draft_update_interval_ms: crate::config::schema::default_lark_draft_update_interval_ms(
            ),
            max_draft_edits: crate::config::schema::default_lark_max_draft_edits(),
        });

        let masked = mask_sensitive_fields(&cfg);
        assert_eq!(masked.proxy.http_proxy.as_deref(), Some(MASKED_SECRET));
        assert_eq!(masked.proxy.https_proxy.as_deref(), Some(MASKED_SECRET));
        assert_eq!(masked.proxy.all_proxy.as_deref(), Some(MASKED_SECRET));
        assert_eq!(
            masked
                .tunnel
                .cloudflare
                .as_ref()
                .map(|value| value.token.as_str()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            masked
                .tunnel
                .ngrok
                .as_ref()
                .map(|value| value.auth_token.as_str()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            masked
                .channels_config
                .wati
                .as_ref()
                .map(|value| value.api_token.as_str()),
            Some(MASKED_SECRET)
        );
        assert_eq!(
            masked
                .channels_config
                .email
                .as_ref()
                .map(|value| value.password.as_str()),
            Some(MASKED_SECRET)
        );
        let masked_feishu = masked
            .channels_config
            .feishu
            .as_ref()
            .expect("feishu config should exist");
        assert_eq!(masked_feishu.app_secret, MASKED_SECRET);
        assert_eq!(masked_feishu.encrypt_key.as_deref(), Some(MASKED_SECRET));
        assert_eq!(
            masked_feishu.verification_token.as_deref(),
            Some(MASKED_SECRET)
        );
    }

    #[test]
    fn hydrate_config_for_save_restores_wati_email_and_feishu_secrets() {
        let mut current = crate::config::Config::default();
        current.proxy.http_proxy = Some("http://user:pass@proxy.internal:8080".to_string());
        current.proxy.https_proxy = Some("https://user:pass@proxy.internal:8443".to_string());
        current.proxy.all_proxy = Some("socks5://user:pass@proxy.internal:1080".to_string());
        current.tunnel.cloudflare = Some(CloudflareTunnelConfig {
            token: "cloudflare-real-token".to_string(),
        });
        current.tunnel.ngrok = Some(NgrokTunnelConfig {
            auth_token: "ngrok-real-token".to_string(),
            domain: Some("zeroclaw.ngrok.app".to_string()),
        });
        current.channels_config.wati = Some(WatiConfig {
            api_token: "wati-real-token".to_string(),
            api_url: "https://live-mt-server.wati.io".to_string(),
            tenant_id: Some("tenant-1".to_string()),
            allowed_numbers: vec!["*".to_string()],
        });
        let mut email = crate::channels::email_channel::EmailConfig::default();
        email.password = "email-real-password".to_string();
        current.channels_config.email = Some(email);
        current.channels_config.feishu = Some(crate::config::FeishuConfig {
            app_id: "cli_app_id".to_string(),
            app_secret: "feishu-real-secret".to_string(),
            encrypt_key: Some("feishu-encrypt-key".to_string()),
            verification_token: Some("feishu-verify-token".to_string()),
            allowed_users: vec!["*".to_string()],
            group_reply: None,
            receive_mode: LarkReceiveMode::Webhook,
            port: Some(42617),
            draft_update_interval_ms: crate::config::schema::default_lark_draft_update_interval_ms(
            ),
            max_draft_edits: crate::config::schema::default_lark_max_draft_edits(),
        });

        let incoming = mask_sensitive_fields(&current);
        let restored = hydrate_config_for_save(incoming, &current);

        assert_eq!(
            restored.proxy.http_proxy.as_deref(),
            Some("http://user:pass@proxy.internal:8080")
        );
        assert_eq!(
            restored.proxy.https_proxy.as_deref(),
            Some("https://user:pass@proxy.internal:8443")
        );
        assert_eq!(
            restored.proxy.all_proxy.as_deref(),
            Some("socks5://user:pass@proxy.internal:1080")
        );
        assert_eq!(
            restored
                .tunnel
                .cloudflare
                .as_ref()
                .map(|value| value.token.as_str()),
            Some("cloudflare-real-token")
        );
        assert_eq!(
            restored
                .tunnel
                .ngrok
                .as_ref()
                .map(|value| value.auth_token.as_str()),
            Some("ngrok-real-token")
        );
        assert_eq!(
            restored
                .channels_config
                .wati
                .as_ref()
                .map(|value| value.api_token.as_str()),
            Some("wati-real-token")
        );
        assert_eq!(
            restored
                .channels_config
                .email
                .as_ref()
                .map(|value| value.password.as_str()),
            Some("email-real-password")
        );
        let restored_feishu = restored
            .channels_config
            .feishu
            .as_ref()
            .expect("feishu config should exist");
        assert_eq!(restored_feishu.app_secret, "feishu-real-secret");
        assert_eq!(
            restored_feishu.encrypt_key.as_deref(),
            Some("feishu-encrypt-key")
        );
        assert_eq!(
            restored_feishu.verification_token.as_deref(),
            Some("feishu-verify-token")
        );
    }
}


#[cfg(test)]
mod app_channel_tests {
    use super::*;
    use crate::config::Config;
    use crate::memory::{Memory, MemoryCategory, MemoryEntry};
    use crate::providers::Provider;
    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{header, HeaderMap, HeaderValue, Request, StatusCode},
        routing::{get, post},
        Router,
    };
    use http_body_util::BodyExt;
    use parking_lot::Mutex;
    use sha2::{Digest, Sha256};
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;
    use tower::ServiceExt;

    #[derive(Default)]
    struct MockMemory;

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }

        async fn store(
            &self,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[derive(Default)]
    struct MockProvider;

    #[async_trait]
    impl Provider for MockProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            _message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok("ok".to_string())
        }
    }

    fn build_state(pairing_required: bool, token: Option<&str>, webhook_limit: u32) -> AppState {
        let tokens = token
            .map(|value| vec![value.to_string()])
            .unwrap_or_default();

        AppState {
            config: Arc::new(Mutex::new(Config::default())),
            provider: Arc::new(MockProvider),
            model: "test-model".into(),
            temperature: 0.0,
            mem: Arc::new(MockMemory),
            auto_save: false,
            webhook_secret_hash: None,
            pairing: Arc::new(crate::security::pairing::PairingGuard::new(
                pairing_required,
                &tokens,
            )),
            trust_forwarded_headers: false,
            rate_limiter: Arc::new(super::super::GatewayRateLimiter::new(
                100,
                webhook_limit,
                1_000,
            )),
            idempotency_store: Arc::new(super::super::IdempotencyStore::new(
                Duration::from_secs(300),
                1_000,
            )),
            whatsapp: None,
            whatsapp_app_secret: None,
            linq: None,
            linq_signing_secret: None,
            nextcloud_talk: None,
            nextcloud_talk_webhook_secret: None,
            wati: None,
            qq: None,
            qq_webhook_enabled: false,
            observer: Arc::new(crate::observability::NoopObserver),
            tools_registry: Arc::new(Vec::new()),
            tools_registry_exec: Arc::new(Vec::new()),
            multimodal: crate::config::MultimodalConfig::default(),
            max_tool_iterations: 10,
            cost_tracker: None,
            event_tx: tokio::sync::broadcast::channel(8).0,
        }
    }

    fn app_router(state: AppState) -> Router {
        Router::new()
            .route("/messages", post(handle_api_app_channel_message))
            .route(
                "/tasks/{task_id}/progress",
                get(handle_api_app_channel_task_progress),
            )
            .route("/system/metrics", get(handle_api_app_channel_system_metrics))
            .route("/stream", get(handle_api_app_channel_stream))
            .with_state(state)
    }

    fn message_payload(content: &str) -> String {
        serde_json::json!({
            "session_id": "sess-1",
            "user_id": "user-1",
            "content": content,
        })
        .to_string()
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    #[tokio::test]
    async fn app_channel_message_requires_auth_when_pairing_enabled() {
        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_test_token"), 100));

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(message_payload("hello")))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn app_channel_message_rejects_missing_content() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(false, None, 100));

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(message_payload("   ")))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn app_channel_message_rejects_missing_session_id() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(false, None, 100));

        let req_body = serde_json::json!({
            "session_id": "   ",
            "user_id": "user-1",
            "content": "hello"
        })
        .to_string();

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(req_body))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn app_channel_message_rejects_missing_user_id() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(false, None, 100));

        let req_body = serde_json::json!({
            "session_id": "sess-1",
            "user_id": "   ",
            "content": "hello"
        })
        .to_string();

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(req_body))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn app_channel_task_progress_returns_404_for_unknown_task() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(false, None, 100));

        let submit_req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(message_payload("create task")))
            .expect("valid request");
        let submit_resp = app.clone().oneshot(submit_req).await.expect("response");
        assert_eq!(submit_resp.status(), StatusCode::ACCEPTED);

        let req = Request::builder()
            .method("GET")
            .uri("/tasks/task-99999999/progress")
            .body(Body::empty())
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn app_channel_message_hits_rate_limit_429() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(false, None, 1));

        let req1 = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(message_payload("msg1")))
            .expect("valid request");
        let resp1 = app.clone().oneshot(req1).await.expect("response");
        assert_eq!(resp1.status(), StatusCode::ACCEPTED);

        let req2 = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(message_payload("msg2")))
            .expect("valid request");
        let resp2 = app.oneshot(req2).await.expect("response");
        assert_eq!(resp2.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn app_channel_stream_ws_handshake_requires_auth() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        let state = build_state(true, Some("zc_ws_token"), 100);
        let headers = HeaderMap::new();

        let result = require_app_channel_auth_with_query(&state, &headers, None);
        assert!(result.is_err());
        let (status, _) = result.expect_err("auth should fail without token");
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn app_channel_stream_ws_handshake_succeeds_with_bearer() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        let state = build_state(true, Some("zc_ws_token"), 100);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer zc_ws_token"),
        );

        let result = require_app_channel_auth_with_query(&state, &headers, None);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn app_channel_stream_ws_handshake_succeeds_with_query_channel_key_sha256() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");

        let raw_key = "k_test_123";
        let expected_hex = hex::encode(Sha256::digest(raw_key.as_bytes()));
        let _set_sha = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY_SHA256", &expected_hex);

        let state = build_state(true, Some("zc_pair_token"), 100);
        let headers = HeaderMap::new();

        let result = require_app_channel_auth_with_query(&state, &headers, Some(raw_key));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn app_channel_stream_ws_handshake_succeeds_with_query_channel_key_raw() {
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        let state = build_state(true, Some("zc_pair_token"), 100);
        let headers = HeaderMap::new();

        let result = require_app_channel_auth_with_query(&state, &headers, Some("raw_secret"));
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn app_channel_http_endpoints_do_not_accept_query_channel_key() {
        // Only WebSocket handshake supports ?channel_key= fallback (web constraint).
        // HTTP endpoints must require X-Channel-Key or Bearer.
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("POST")
            .uri("/messages?channel_key=raw_secret")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(message_payload("hello")))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn app_channel_auth_sha256_accepts_correct_key_via_header() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");

        let raw_key = "k_test_123";
        let expected_hex = hex::encode(Sha256::digest(raw_key.as_bytes()));
        let _set_sha = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY_SHA256", &expected_hex);

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header("X-Channel-Key", HeaderValue::from_static(raw_key))
            .body(Body::from(message_payload("hello")))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn app_channel_auth_sha256_rejects_wrong_key() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");

        let expected_hex = hex::encode(Sha256::digest(b"correct"));
        let _set_sha = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY_SHA256", &expected_hex);

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header("X-Channel-Key", HeaderValue::from_static("wrong"))
            .body(Body::from(message_payload("hello")))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        let err = body.get("error").and_then(serde_json::Value::as_str).unwrap_or("");
        assert!(!err.contains("wrong"));
        assert!(!err.contains("correct"));
    }

    #[tokio::test]
    async fn app_channel_auth_raw_still_works_when_configured() {
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header("X-Channel-Key", HeaderValue::from_static("raw_secret"))
            .body(Body::from(message_payload("hello")))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn app_channel_system_metrics_requires_auth_when_pairing_enabled() {
        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("GET")
            .uri("/system/metrics?window=1h")
            .body(Body::empty())
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn app_channel_system_metrics_does_not_accept_query_channel_key() {
        // HTTP endpoints must require X-Channel-Key or Bearer.
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("GET")
            .uri("/system/metrics?window=1h&channel_key=raw_secret")
            .body(Body::empty())
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn app_channel_system_metrics_accepts_header_channel_key_raw() {
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("GET")
            .uri("/system/metrics?window=1h")
            .header("X-Channel-Key", HeaderValue::from_static("raw_secret"))
            .body(Body::empty())
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn app_channel_task_progress_does_not_accept_query_channel_key() {
        // HTTP endpoints must require X-Channel-Key or Bearer.
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("GET")
            .uri("/tasks/task-99999999/progress?channel_key=raw_secret")
            .body(Body::empty())
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn app_channel_message_rejects_invalid_metadata_shape() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(false, None, 100));

        let req_body = serde_json::json!({
            "session_id": "sess-1",
            "user_id": "user-1",
            "content": "hello",
            "metadata": {
                "ok": "yes",
                "bad": 123
            }
        })
        .to_string();

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(req_body))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(
            body.get("error").and_then(serde_json::Value::as_str),
            Some("Invalid JSON body for app-channel message")
        );
    }

    #[tokio::test]
    async fn app_channel_system_metrics_clamps_step_sec_boundary() {
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let low_req = Request::builder()
            .method("GET")
            .uri("/system/metrics?window=1h&step_sec=0")
            .header("X-Channel-Key", HeaderValue::from_static("raw_secret"))
            .body(Body::empty())
            .expect("valid request");

        let low_resp = app.clone().oneshot(low_req).await.expect("response");
        assert_eq!(low_resp.status(), StatusCode::OK);
        let low_body_bytes = low_resp.into_body().collect().await.expect("body").to_bytes();
        let low_body: serde_json::Value = serde_json::from_slice(&low_body_bytes).expect("json");
        assert_eq!(low_body.get("step_sec").and_then(serde_json::Value::as_u64), Some(1));

        let high_req = Request::builder()
            .method("GET")
            .uri("/system/metrics?window=1h&step_sec=9999")
            .header("X-Channel-Key", HeaderValue::from_static("raw_secret"))
            .body(Body::empty())
            .expect("valid request");

        let high_resp = app.oneshot(high_req).await.expect("response");
        assert_eq!(high_resp.status(), StatusCode::OK);
        let high_body_bytes = high_resp.into_body().collect().await.expect("body").to_bytes();
        let high_body: serde_json::Value = serde_json::from_slice(&high_body_bytes).expect("json");
        assert_eq!(
            high_body.get("step_sec").and_then(serde_json::Value::as_u64),
            Some(300)
        );
    }

    #[tokio::test]
    async fn app_channel_system_metrics_invalid_window_returns_standard_error_shape() {
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("GET")
            .uri("/system/metrics?window=2h")
            .header("X-Channel-Key", HeaderValue::from_static("raw_secret"))
            .body(Body::empty())
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        let err = body.get("error").and_then(serde_json::Value::as_str).unwrap_or("");
        assert!(err.contains("Invalid window"));
    }

    #[test]
    fn app_channel_stream_progress_interval_clamps_to_min_and_max() {
        let mut query = std::collections::BTreeMap::new();

        query.insert("progress_interval_sec".to_string(), "1".to_string());
        let low = parse_query_interval(
            &query,
            "progress_interval_sec",
            APP_STREAM_PUSH_INTERVAL_SECS,
            APP_STREAM_MIN_INTERVAL_SECS,
            APP_STREAM_MAX_INTERVAL_SECS,
        );
        assert_eq!(low, APP_STREAM_MIN_INTERVAL_SECS);

        query.insert("progress_interval_sec".to_string(), "999".to_string());
        let high = parse_query_interval(
            &query,
            "progress_interval_sec",
            APP_STREAM_PUSH_INTERVAL_SECS,
            APP_STREAM_MIN_INTERVAL_SECS,
            APP_STREAM_MAX_INTERVAL_SECS,
        );
        assert_eq!(high, APP_STREAM_MAX_INTERVAL_SECS);
    }

    #[test]
    fn app_channel_stream_summary_interval_clamps_and_defaults_on_invalid() {
        let mut query = std::collections::BTreeMap::new();

        query.insert("summary_interval_sec".to_string(), "5".to_string());
        let low = parse_query_interval(
            &query,
            "summary_interval_sec",
            APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MIN_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MAX_SUMMARY_INTERVAL_SECS,
        );
        assert_eq!(low, APP_STREAM_MIN_SUMMARY_INTERVAL_SECS);

        query.insert("summary_interval_sec".to_string(), "9999".to_string());
        let high = parse_query_interval(
            &query,
            "summary_interval_sec",
            APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MIN_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MAX_SUMMARY_INTERVAL_SECS,
        );
        assert_eq!(high, APP_STREAM_MAX_SUMMARY_INTERVAL_SECS);

        query.insert("summary_interval_sec".to_string(), "nan".to_string());
        let invalid = parse_query_interval(
            &query,
            "summary_interval_sec",
            APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MIN_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MAX_SUMMARY_INTERVAL_SECS,
        );
        assert_eq!(invalid, APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS);

        query.remove("summary_interval_sec");
        let missing = parse_query_interval(
            &query,
            "summary_interval_sec",
            APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MIN_SUMMARY_INTERVAL_SECS,
            APP_STREAM_MAX_SUMMARY_INTERVAL_SECS,
        );
        assert_eq!(missing, APP_STREAM_DEFAULT_SUMMARY_INTERVAL_SECS);
    }

    #[tokio::test]
    async fn app_channel_error_responses_keep_error_field_shape_consistent() {
        let _guard = env_lock();
        let _unset_sha = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY_SHA256");
        let _set_raw = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY", "raw_secret");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        // 401: missing app-channel auth
        let unauthorized_req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .body(Body::from(message_payload("hello")))
            .expect("valid request");
        let unauthorized_resp = app
            .clone()
            .oneshot(unauthorized_req)
            .await
            .expect("response");
        assert_eq!(unauthorized_resp.status(), StatusCode::UNAUTHORIZED);
        let unauthorized_body_bytes = unauthorized_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let unauthorized_body: serde_json::Value =
            serde_json::from_slice(&unauthorized_body_bytes).expect("json");
        assert!(unauthorized_body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some());

        // 400: invalid metrics window
        let bad_window_req = Request::builder()
            .method("GET")
            .uri("/system/metrics?window=oops")
            .header("X-Channel-Key", HeaderValue::from_static("raw_secret"))
            .body(Body::empty())
            .expect("valid request");
        let bad_window_resp = app.clone().oneshot(bad_window_req).await.expect("response");
        assert_eq!(bad_window_resp.status(), StatusCode::BAD_REQUEST);
        let bad_window_body_bytes = bad_window_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let bad_window_body: serde_json::Value =
            serde_json::from_slice(&bad_window_body_bytes).expect("json");
        assert!(bad_window_body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some());

        // 404: unknown task progress
        let not_found_req = Request::builder()
            .method("GET")
            .uri("/tasks/task-does-not-exist/progress")
            .header("X-Channel-Key", HeaderValue::from_static("raw_secret"))
            .body(Body::empty())
            .expect("valid request");
        let not_found_resp = app.oneshot(not_found_req).await.expect("response");
        assert_eq!(not_found_resp.status(), StatusCode::NOT_FOUND);
        let not_found_body_bytes = not_found_resp
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let not_found_body: serde_json::Value =
            serde_json::from_slice(&not_found_body_bytes).expect("json");
        assert!(not_found_body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .is_some());
    }

    #[tokio::test]
    async fn app_channel_auth_sha256_misconfig_returns_500_and_generic_error() {
        let _guard = env_lock();
        let _unset_raw = EnvVarGuard::unset("ZEROCLAW_APP_CHANNEL_KEY");

        // invalid hex / wrong length
        let _set_sha = EnvVarGuard::set("ZEROCLAW_APP_CHANNEL_KEY_SHA256", "not-hex");

        reset_app_task_state_for_tests();
        let app = app_router(build_state(true, Some("zc_pair_token"), 100));

        let req = Request::builder()
            .method("POST")
            .uri("/messages")
            .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header("X-Channel-Key", HeaderValue::from_static("anything"))
            .body(Body::from(message_payload("hello")))
            .expect("valid request");

        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let body_bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).expect("json");
        assert_eq!(body.get("error").and_then(serde_json::Value::as_str), Some("Server misconfiguration"));
    }
}
