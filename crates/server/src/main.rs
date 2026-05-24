//! immich-extended binary entry point.

use std::io;

use anyhow::{Context, Result};
use server::config::Config;
use tokio::{net::TcpListener, signal};
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env().context("loading configuration from env")?;

    init_tracing(&cfg.log_level)?;

    info!(
        version = server::version(),
        http_bind = %cfg.http_bind,
        data_dir = %cfg.data_dir.display(),
        engine_version = engine::version(),
        immich_client_version = immich_client::version(),
        yolo_version = yolo::version(),
        common_version = common::version(),
        "immich-extended starting"
    );

    let app = server::router();

    let listener = TcpListener::bind(cfg.http_bind)
        .await
        .with_context(|| format!("binding HTTP listener to {}", cfg.http_bind))?;

    let local = listener
        .local_addr()
        .context("reading bound HTTP listener address")?;
    info!(local_addr = %local, "HTTP listener bound, serving");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("axum::serve terminated with an error")?;

    info!("immich-extended stopped");
    Ok(())
}

fn init_tracing(log_level: &str) -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level));

    let json_layer = fmt::layer().json().with_writer(io::stdout);

    tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .try_init()
        .map_err(|e| anyhow::anyhow!("tracing subscriber init failed: {e}"))?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = signal::ctrl_c().await {
            tracing::error!(error = %err, "failed to install SIGINT handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(err) => {
                tracing::error!(error = %err, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => { info!("received SIGINT, shutting down"); }
        _ = terminate => { info!("received SIGTERM, shutting down"); }
    }
}
