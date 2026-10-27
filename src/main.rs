use clap::{Parser, Subcommand};
use enterprise_rust_backend::{
    bootstrap::{AppState, build_app},
    config::Config,
    database,
};
use std::{net::SocketAddr, process::ExitCode, time::Duration};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server. Schema migrations are a separate deployment step.
    Serve,
    /// Apply embedded SQL migrations using DATABASE_URL_UNPOOLED when configured.
    Migrate,
    /// Verify PostgreSQL connectivity without modifying data.
    CheckDb,
    /// Seed an initial demo administrator test user into the database.
    Seed,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Err(error) = dotenvy::dotenv()
        && !error.not_found()
    {
        eprintln!("Unable to parse .env configuration");
        return ExitCode::FAILURE;
    }
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .json()
        .with_target(false)
        .init();
    let config = match Config::from_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "startup rejected");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = run(cli.command.unwrap_or(Command::Serve), config).await {
        tracing::error!(%error, "application stopped");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

async fn run(command: Command, config: Config) -> Result<(), Box<dyn std::error::Error>> {
    if matches!(command, Command::Migrate) {
        database::migrate(&config).await?;
        tracing::info!("database migrations applied");
        return Ok(());
    }
    let pool = database::connect(&config).await?;
    if matches!(command, Command::CheckDb) {
        database::check(&pool).await?;
        tracing::info!("database connection verified");
        pool.close().await;
        return Ok(());
    }
    if matches!(command, Command::Seed) {
        let user = database::seed(&pool).await?;
        tracing::info!(id = %user.id, email = %user.email, role = %user.role.as_str(), "demo test user seeded in database");
        pool.close().await;
        return Ok(());
    }
    let address = SocketAddr::new(config.host, config.port);
    let state = AppState::new(pool.clone(), config).await?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "HTTP server listening");
    axum::serve(
        listener,
        build_app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    let _ = tokio::time::timeout(Duration::from_secs(10), pool.close()).await;
    tracing::info!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
    tracing::info!("shutdown requested; draining in-flight requests");
}
