use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use dochestra_engine::build_app;
use dochestra_engine::config::Config;
use dochestra_engine::db::runs;
use dochestra_engine::nodes::EngineRunner;
use dochestra_engine::state::AppState;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

const HEALTH_POLL_INTERVAL: Duration = Duration::from_secs(2);
const HEALTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Config::from_env().context("failed to load configuration")?;

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("failed to connect to database")?;
    sqlx::migrate!("./migrations")
        .run(&db)
        .await
        .context("failed to run migrations")?;
    tracing::info!("database migrations applied");

    let orphaned = runs::fail_orphaned_runs(&db)
        .await
        .context("failed to clean up interrupted runs")?;
    if orphaned > 0 {
        tracing::info!(count = orphaned, "marked interrupted runs as failed");
    }

    tokio::fs::create_dir_all(&config.data_dir)
        .await
        .with_context(|| format!("failed to create data dir {}", config.data_dir.display()))?;

    let http = reqwest::Client::builder()
        .build()
        .context("failed to build http client")?;

    if config.extractor_health_required {
        wait_for_extractor(&http, &config).await?;
    } else {
        tracing::warn!("EXTRACTOR_HEALTH_REQUIRED=false: skipping extractor health wait");
    }

    let bind = config.bind;
    let runner = EngineRunner::new(db.clone(), Arc::new(config.clone()), http.clone());
    let state = AppState::new(db, config, http, Arc::new(runner));
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .with_context(|| format!("failed to bind {bind}"))?;
    tracing::info!(%bind, "engine listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;
    tracing::info!("engine stopped");
    Ok(())
}

/// Polls `{EXTRACTOR_URL}/health` until it returns 200 or the overall timeout elapses.
async fn wait_for_extractor(http: &reqwest::Client, config: &Config) -> anyhow::Result<()> {
    let url = format!("{}/health", config.extractor_url);
    let deadline = Instant::now() + config.extractor_health_timeout;
    tracing::info!(%url, timeout = ?config.extractor_health_timeout, "waiting for extractor");
    loop {
        match http.get(&url).timeout(HEALTH_REQUEST_TIMEOUT).send().await {
            Ok(resp) if resp.status().is_success() => {
                tracing::info!("extractor is healthy");
                return Ok(());
            }
            Ok(resp) => tracing::debug!(status = %resp.status(), "extractor not ready"),
            Err(err) => tracing::debug!(error = %err, "extractor not reachable"),
        }
        if Instant::now() + HEALTH_POLL_INTERVAL > deadline {
            bail!(
                "extractor did not become healthy within {:?} ({url})",
                config.extractor_health_timeout
            );
        }
        tokio::time::sleep(HEALTH_POLL_INTERVAL).await;
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
