//! immich-extended binary entry point.
//!
//! Dispatches to either the long-running HTTP server (`serve`, default) or a
//! short-lived admin command (`admin create-user ...`). Both code paths share
//! the same config loader, tracing init, and migration runner so a fresh DB
//! works identically whichever command runs first.

use std::io;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use common::db;
use server::{
    activity::ActivityBus,
    admin::{create_user, CreateUserError},
    auth::oidc::OidcClient,
    config::Config,
    engine_scheduler::Scheduler,
    indexer::{Indexer, OnSweepFn},
    matcher::Matcher,
    rules::resolver::ImmichResourceResolver,
    AppState,
};
use tokio::{net::TcpListener, signal};
use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

#[derive(Debug, Parser)]
#[command(
    name = "immich-extended",
    version,
    about = "Rule-driven Immich automation"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the HTTP server (default when no subcommand is given).
    Serve,
    /// Administrative operations that do not require the server to be running.
    Admin {
        #[command(subcommand)]
        cmd: AdminCmd,
    },
}

#[derive(Debug, Subcommand)]
enum AdminCmd {
    /// Create a new local-credentials user.
    CreateUser {
        #[arg(long)]
        email: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        display_name: Option<String>,
        /// Grant platform-admin privileges (is_admin = 1).
        #[arg(long)]
        admin: bool,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let cfg = match Config::from_env().context("loading configuration from env") {
        Ok(c) => c,
        Err(err) => {
            eprintln!("config error: {err:#}");
            return ExitCode::from(1);
        }
    };

    if let Err(err) = init_tracing(&cfg.log_level) {
        eprintln!("tracing init failed: {err:#}");
        return ExitCode::from(1);
    }

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => match run_serve(cfg).await {
            Ok(()) => ExitCode::from(0),
            Err(err) => {
                tracing::error!(error = %format!("{err:#}"), "server exited with error");
                ExitCode::from(1)
            }
        },
        Command::Admin { cmd } => run_admin(cfg, cmd).await,
    }
}

async fn run_serve(cfg: Config) -> Result<()> {
    info!(
        version = server::version(),
        http_bind = %cfg.http_bind,
        data_dir = %cfg.data_dir.display(),
        database_url = %cfg.database_url,
        engine_version = engine::version(),
        immich_client_version = immich_client::version(),
        yolo_version = yolo::version(),
        common_version = common::version(),
        "immich-extended starting"
    );

    tokio::fs::create_dir_all(&cfg.data_dir)
        .await
        .with_context(|| format!("creating data dir {}", cfg.data_dir.display()))?;

    let pool = db::open_pool(&cfg.database_url)
        .await
        .with_context(|| format!("opening sqlite pool at {}", cfg.database_url))?;
    db::run_migrations(&pool)
        .await
        .context("running sqlx migrations")?;
    info!("migrations applied");

    let oidc_client: Option<OidcClient> = match cfg.oidc.as_ref() {
        Some(oidc_cfg) => {
            info!(
                issuer_url = %oidc_cfg.issuer_url,
                client_id = %oidc_cfg.client_id,
                "OIDC enabled, running discovery"
            );
            let client = OidcClient::from_config(oidc_cfg)
                .await
                .context("OIDC discovery (set OIDC_ISSUER_URL='' to disable)")?;
            info!("OIDC discovery succeeded, login routes enabled");
            Some(client)
        }
        None => {
            warn!("OIDC disabled — no OIDC_ISSUER_URL set");
            None
        }
    };

    // Per-rule Immich clients are built inside the poll cycle from each
    // rule owner's stored `immich_api_keys.base_url`, so the scheduler
    // doesn't need a global Immich URL — it just needs the master key to
    // decrypt the stored secret. The `data_dir` plumbs through to the YOLO
    // dispatch (M5-T6) so the model file at `data_dir/models/yolo.onnx`
    // resolves at inference time.
    // Live-activity ring buffer (POSTSHIP-T33). One process-wide buffer shared
    // by the indexer (per-asset "Indexed" events), the rule-cycle tick fn
    // (per-decision "Matched"/"Skipped"/"AlbumAdd"), and the
    // `/me/activity/stream` endpoint. Constructed before the scheduler + indexer
    // so both can publish into the same buffer the endpoint reads.
    let activity = Arc::new(ActivityBus::new());

    // Per-rule poll Scheduler is still constructed + started below for
    // branch-safety; its per-rule timers are formally retired in T42. CRUD
    // handlers no longer reconcile it — the rule lifecycle now drives matching
    // through `Matcher::on_rule_activated` instead.
    let scheduler = Arc::new(Scheduler::new(
        pool.clone(),
        cfg.master_key.clone(),
        cfg.data_dir.clone(),
        activity.clone(),
    ));

    // Event-driven matcher service (POSTSHIP-T41, design §5.2). One thin
    // wiring point — capturing pool + master_key + data_dir + activity — for the
    // three matching triggers: the rule lifecycle (`on_rule_activated`, held in
    // AppState), the indexer sweep hook (`match_assets`, below), and the hourly
    // safety sweep (`safety_sweep`, wired in T42).
    let matcher = Arc::new(Matcher::new(
        pool.clone(),
        cfg.master_key.clone(),
        cfg.data_dir.clone(),
        activity.clone(),
    ));

    // Event-driven matching seam (POSTSHIP-T40, design §4.2). After each user
    // sweep the indexer hands that sweep's touched asset_ids to pass (b)
    // (`Matcher::match_assets`): evaluate exactly those assets against all of
    // that user's active rules and reconcile albums. The indexer itself stays
    // storage-only (no engine/YOLO/data_dir dependency) — the matcher carries
    // those. A single rule's failure is logged and skipped inside `match_assets`;
    // the `MatchError` surfaced here is only the outer load (active-rule set /
    // candidate rows), logged so a transient DB hiccup never aborts the sweep
    // loop.
    let on_sweep: OnSweepFn = {
        let matcher = matcher.clone();
        Arc::new(move |user_id: String, touched_ids: Vec<String>| {
            let matcher = matcher.clone();
            Box::pin(async move {
                if let Err(err) = matcher.match_assets(&user_id, &touched_ids).await {
                    tracing::error!(
                        user_id = %user_id,
                        error = %err,
                        "event-driven match after sweep failed",
                    );
                }
            })
        })
    };

    // Background whole-library pre-processing indexer (POSTSHIP-T28). One
    // process-wide task that keeps `asset_index` fresh for every keyed user so
    // rule matching can scan locally. Built per-user from stored keys, so — like
    // the scheduler — it needs only the master key, not a global Immich URL.
    // `with_on_sweep` attaches the event-driven matcher hook (T40). Held here
    // (not in AppState): no CRUD hook reaches it in this cycle.
    let indexer = Arc::new(
        Indexer::new(pool.clone(), cfg.master_key.clone(), activity.clone())
            .with_on_sweep(on_sweep),
    );

    let state = AppState {
        db: pool.clone(),
        session: cfg.session.clone(),
        master_key: cfg.master_key.clone(),
        oidc: Arc::new(oidc_client),
        resolver: Arc::new(ImmichResourceResolver {
            db: pool.clone(),
            master_key: cfg.master_key.clone(),
        }),
        matcher: matcher.clone(),
        activity: activity.clone(),
    };

    scheduler
        .clone()
        .start()
        .await
        .context("starting per-rule scheduler")?;

    indexer.clone().start().await;

    let app = server::router(state, cfg.web_dist_dir.clone());

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

    scheduler.stop().await;
    indexer.stop().await;
    info!("immich-extended stopped");
    pool.close().await;
    Ok(())
}

async fn run_admin(cfg: Config, cmd: AdminCmd) -> ExitCode {
    if let Err(err) = tokio::fs::create_dir_all(&cfg.data_dir).await {
        eprintln!(
            "failed to create data dir {}: {err:#}",
            cfg.data_dir.display()
        );
        return ExitCode::from(1);
    }

    let pool = match db::open_pool(&cfg.database_url).await {
        Ok(p) => p,
        Err(err) => {
            eprintln!("failed to open database at {}: {err:#}", cfg.database_url);
            return ExitCode::from(1);
        }
    };
    if let Err(err) = db::run_migrations(&pool).await {
        eprintln!("failed to run migrations: {err:#}");
        pool.close().await;
        return ExitCode::from(1);
    }

    let exit = match cmd {
        AdminCmd::CreateUser {
            email,
            password,
            display_name,
            admin,
        } => match create_user(&pool, &email, &password, display_name.as_deref(), admin).await {
            Ok(user_id) => {
                println!("{user_id}");
                ExitCode::from(0)
            }
            Err(CreateUserError::DuplicateEmail(e)) => {
                eprintln!("a user with email {e:?} already exists");
                ExitCode::from(2)
            }
            Err(err) => {
                eprintln!("failed to create user: {err:#}");
                ExitCode::from(1)
            }
        },
    };

    pool.close().await;
    exit
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
