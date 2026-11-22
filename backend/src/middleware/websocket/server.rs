/*
 * Copyright (C) 2026 Lachlan
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as
 * published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */
use std::env;
use crate::app::models::client_event::{ClientInput, ClientKey, WireInput};
use crate::app::models::sessions::shared_sessions::SharedSessions;
use axum::extract::ConnectInfo;
use axum::http::Request;
use axum::http::header::AUTHORIZATION;
use axum::middleware::{Next, from_fn};
use axum::response::{IntoResponse, Response};
use axum::{
    Router,
    extract::{
        Json, Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use axum_governor::{GovernorConfigBuilder, GovernorLayer, Quota, extractor::SmartIp, nz};
use color_eyre::Result;
use ipnet::IpNet;
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::timeout;
use tracing::{Instrument, info_span};
use uuid::Uuid;

static HTTP_REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);
static HTTP_REQUEST_DURATION_MICROSECONDS_TOTAL: AtomicU64 = AtomicU64::new(0);
static SESSIONS_CREATED_TOTAL: AtomicU64 = AtomicU64::new(0);
static WEBSOCKET_CONNECTIONS_ACTIVE: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize)]
struct CreateSessionResponse {
    session_id: String,
}

pub async fn req_logger(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let ip = addr.ip();

    let ua = req
        .headers()
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown");

    tracing::info!(
        ip = %ip,
        user_agent = %ua,
        method = %req.method(),
        uri = %req.uri(),
        "request received"
    );

    let started = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let span = tracing::info_span!("http.request", method = %method, path = %path);
    let response = next.run(req).instrument(span).await;
    let elapsed = started.elapsed();
    HTTP_REQUESTS_TOTAL.fetch_add(1, Ordering::Relaxed);
    HTTP_REQUEST_DURATION_MICROSECONDS_TOTAL
        .fetch_add(elapsed.as_micros() as u64, Ordering::Relaxed);
    tracing::info!(
        ip = %ip,
        method = %method,
        path = %path,
        status = response.status().as_u16(),
        duration_ms = elapsed.as_secs_f64() * 1000.0,
        "request completed"
    );
    response
}

pub async fn start_server(sessions: SharedSessions) -> Result<()> {
    let trusted_proxies: Vec<IpNet> = match env::var("APP_ENV").as_deref() {
        Ok("development") => vec!["0.0.0.0/0".parse::<IpNet>()?],
        _ => vec!["172.20.0.0/16".parse::<IpNet>()?], // Fail secure
    };
    let cfg = GovernorConfigBuilder::default()
        .with_extractor(
            SmartIp::new().with_trusted_proxies(trusted_proxies),
        )
        .expect_connect_info()
        .quota_default(Quota::requests_per_minute(nz!(10u32)))
        .finish()?;
    let api = Router::new()
        .route("/session", post(create_session))
        .route("/ws/{id}", get(ws_handler))
        .route("/metrics", get(metrics_handler))
        .layer(GovernorLayer::new(cfg));
    let app = Router::new()
        .route("/health", get(health_handler))
        .nest("/api", api)
        .with_state(sessions)
        .layer(from_fn(req_logger));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:4000").await?;
    tracing::info!(address = %listener.local_addr()?, "Server listening!!");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn health_handler() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(serde_json::json!({ "status": "ok" })))
}

async fn create_session(State(sessions): State<SharedSessions>) -> Json<CreateSessionResponse> {
    tracing::debug!("POST /api/session");
    SESSIONS_CREATED_TOTAL.fetch_add(1, Ordering::Relaxed);
    let shared = sessions.clone();
    let mut manager = sessions.write().await;
    let id = manager.create_session(shared);
    Json(CreateSessionResponse {
        session_id: id.to_string(),
    })
}

async fn metrics_handler(headers: HeaderMap, State(sessions): State<SharedSessions>) -> Response {
    let expected = match std::env::var("METRICS_TOKEN") {
        Ok(token) if !token.is_empty() => format!("Bearer {token}"),
        _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(expected.as_str())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let active_sessions = sessions.read().await.len();
    let body = format!(
        "# HELP tui_http_requests_total Total HTTP requests handled.\n# TYPE tui_http_requests_total counter\ntui_http_requests_total {}\n# HELP tui_http_request_duration_seconds_sum Total HTTP request time in seconds.\n# TYPE tui_http_request_duration_seconds_sum counter\ntui_http_request_duration_seconds_sum {:.6}\n# HELP tui_sessions_created_total Total TUI sessions created.\n# TYPE tui_sessions_created_total counter\ntui_sessions_created_total {}\n# HELP tui_sessions_active Current TUI sessions.\n# TYPE tui_sessions_active gauge\ntui_sessions_active {}\n# HELP tui_websocket_connections_active Current WebSocket connections.\n# TYPE tui_websocket_connections_active gauge\ntui_websocket_connections_active {}\n",
        HTTP_REQUESTS_TOTAL.load(Ordering::Relaxed),
        HTTP_REQUEST_DURATION_MICROSECONDS_TOTAL.load(Ordering::Relaxed) as f64 / 1_000_000.0,
        SESSIONS_CREATED_TOTAL.load(Ordering::Relaxed),
        active_sessions,
        WEBSOCKET_CONNECTIONS_ACTIVE.load(Ordering::Relaxed),
    );
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        body,
    )
        .into_response()
}

async fn ws_handler(
    Path(id): Path<String>,
    State(sessions): State<SharedSessions>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let id = match Uuid::parse_str(&id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    tracing::debug!(id = ?id, "GET /api/ws/{id}");
    let sessions = sessions.read().await;
    let session = match sessions.get_session(&id) {
        Some(session) => session,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let rx = session.output_tx.subscribe();
    let tx = session.input_tx.clone();

    ws.on_upgrade(move |socket| {
        handle_socket(socket, rx, tx).instrument(info_span!("socket", session_id = %id))
    })
    .into_response()
}

async fn handle_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<Vec<u8>>,
    tx: UnboundedSender<ClientInput>,
) {
    WEBSOCKET_CONNECTIONS_ACTIVE.fetch_add(1, Ordering::Relaxed);
    tracing::debug!("ws upgrade");
    loop {
        tokio::select! {
            // tx to client
            result = rx.recv() => {
                match result {
                    Ok(bytes) => {
                        tracing::trace!(msg = ?bytes, "tx to client");
                        if let Err(e) = socket.send(Message::Binary(bytes.into())).await {
                            tracing::warn!(error = %e, "Error sending client message");
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(messages = n, "Broadcast receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::error!("Broadcast channel closed");
                        break;
                    }
                }
            }

            // rx from client
            res = timeout(Duration::from_secs(180), socket.recv()) => {
                match res {
                    Ok(msg) => {
                        tracing::trace!(msg = ?msg, "rx from client");
                        match msg {
                            Some(Ok(Message::Text(text))) => {
                                let text = text.to_string();
                                match parse_input(&text) {
                                    Some(input) => {
                                        if let Err(err) = tx.send(input) {
                                            tracing::warn!(%err, "Error forwarding client input");
                                            break;
                                        };
                                    }

                                    None => {
                                        tracing::warn!(message = %text, "Invalid client message");
                                    }
                                }
                            }
                            Some(Ok(Message::Binary(_))) => {
                                tracing::debug!("Ignoring binary message");
                            }
                            Some(Ok(Message::Close(_))) => {
                                tracing::info!("Client requested close");
                                break;
                            }
                            Some(Err(err)) => {
                                tracing::warn!(%err, "Websocket error");
                                break;
                            }
                            None => {
                                tracing::info!("Websocket stream ended");
                                break;
                            }
                            _ => {
                                tracing::debug!("Ignoring unknown websocket message");
                            }
                        }
                    }
                    Err(t) => {
                        tracing::info!(elapsed=%t,"Client timed out after inactivity");
                        break;
                    }
                }
            }
        }
    }
    tracing::debug!("ws disconnect");
    if let Err(err) = tx.send(ClientInput::Disconnect) {
        tracing::warn!(%err, "Error sending disconnect message");
    };
    WEBSOCKET_CONNECTIONS_ACTIVE.fetch_sub(1, Ordering::Relaxed);
}

fn parse_input(text: &String) -> Option<ClientInput> {
    let msg: WireInput = serde_json::from_str(&text).ok()?;

    match msg {
        WireInput::Key { key } => {
            let ck = match key.as_str() {
                "Enter" => ClientKey::Enter,
                "Escape" => ClientKey::Escape,
                "ArrowLeft" => ClientKey::ArrowLeft,
                "ArrowRight" => ClientKey::ArrowRight,
                "ArrowUp" => ClientKey::ArrowUp,
                "ArrowDown" => ClientKey::ArrowDown,
                "Tab" => ClientKey::Tab,
                c if c.len() == 1 => ClientKey::Char(c.chars().next().unwrap()),
                _ => return None,
            };

            Some(ClientInput::Key(ck))
        }

        WireInput::Resize { cols, rows } => Some(ClientInput::Resize { cols, rows }),
        WireInput::Ready => Some(ClientInput::Ready),
    }
}
