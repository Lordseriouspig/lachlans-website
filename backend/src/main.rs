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

pub mod app;
pub mod middleware;
pub mod tui;

use color_eyre::Result;
use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use std::env;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;
use crate::app::api_poller::spawn_poller;
use crate::app::models::api::api_state::{ApiState, SharedApiState};
use crate::app::models::api::stats::StatsConf;
use crate::app::models::sessions::manager::SessionManager;
use crate::middleware::websocket::server::start_server;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(debug_assertions)]
    dotenvy::dotenv().ok();

    color_eyre::install()?;

    let tracer_provider = if let Ok(endpoint) = env::var("OTEL_EXPORTER_OTLP_ENDPOINT") {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()?;
        let provider = SdkTracerProvider::builder()
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("tui-backend")
                    .build(),
            )
            .with_batch_exporter(exporter)
            .build();
        global::set_tracer_provider(provider.clone());
        Some(provider)
    } else {
        None
    };

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer().json())
        .with(tracing_opentelemetry::layer())
        .try_init()?;

    tracing::info!(debug=%cfg!(debug_assertions), "Starting!");

    let api_state: SharedApiState =
        Arc::new(RwLock::new(ApiState::default()));
    let poller_state = api_state.clone();
    let api_client = reqwest::Client::new();

    let stats_cfg = StatsConf {
        base_url: env::var("HACKATIME_URL")
            .unwrap_or_else(|_| "https://hackatime.hackclub.com".to_string()),
        username: env::var("HACKATIME_USERNAME").ok(),
        api_key: env::var("HACKATIME_STATS_API_KEY").ok(),
    };

    let poller = tokio::spawn(
        async move {
            spawn_poller(poller_state, api_client, stats_cfg).await;
        }
            .instrument(tracing::info_span!("API Poller")),
    );

    let sessions = Arc::new(RwLock::new(
        SessionManager::new(api_state.clone()),
    ));

    start_server(sessions).await?;

    poller.abort();

    if let Some(provider) = tracer_provider {
        provider.shutdown()?;
    }

    Ok(())
}
