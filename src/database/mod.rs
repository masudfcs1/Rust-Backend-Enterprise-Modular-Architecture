use crate::{config::Config, shared::AppError};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::time::Duration;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub async fn connect(config: &Config) -> Result<PgPool, AppError> {
    open(
        &config.database_url,
        config.database_max_connections,
        config.database_acquire_timeout_seconds,
    )
    .await
}

async fn open(url: &str, max: u32, timeout: u64) -> Result<PgPool, AppError> {
    let options: PgConnectOptions = url.parse().map_err(|_| AppError::Internal)?;
    // Never log SQL, bindings, or connection strings; statement cache disabled for poolers.
    let options = options
        .application_name("enterprise-rust-backend")
        .statement_cache_capacity(0)
        .disable_statement_logging();
    PgPoolOptions::new()
        .max_connections(max)
        .min_connections(0)
        .acquire_timeout(Duration::from_secs(timeout))
        .idle_timeout(Duration::from_secs(60))
        .max_lifetime(Duration::from_secs(1800))
        .connect_with(options)
        .await
        .map_err(AppError::from)
}

/// Explicit deployment step, using a direct endpoint for advisory-lock safety.
pub async fn migrate(config: &Config) -> Result<(), AppError> {
    let url = config
        .database_url_unpooled
        .as_deref()
        .unwrap_or(&config.database_url);
    let parsed = url::Url::parse(url).map_err(|_| AppError::Internal)?;
    if parsed.host_str().is_some_and(|h| h.contains("-pooler.")) {
        return Err(AppError::Validation(
            "Set DATABASE_URL_UNPOOLED to the direct endpoint for migrations".into(),
        ));
    }
    let pool = open(url, 1, config.database_acquire_timeout_seconds).await?;
    let result = MIGRATOR.run(&pool).await.map_err(|_| {
        tracing::error!("database migration failed; check schema and migration history");
        AppError::Internal
    });
    pool.close().await;
    result
}

pub async fn check(pool: &PgPool) -> Result<(), AppError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await?;
    Ok(())
}

/// Readiness includes schema state, so an unmigrated service never receives traffic.
pub async fn ready(pool: &PgPool) -> Result<(), AppError> {
    let present: bool = sqlx::query_scalar(
        "SELECT to_regclass('_sqlx_migrations') IS NOT NULL
            AND to_regclass('users') IS NOT NULL
            AND to_regclass('auth_sessions') IS NOT NULL
            AND to_regclass('auth_refresh_tokens') IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    if !present {
        return Err(AppError::Unavailable);
    }
    let applied: Vec<(i64, bool, Vec<u8>)> =
        sqlx::query_as("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await?;
    for migration in MIGRATOR.iter() {
        if !applied.iter().any(|(version, success, checksum)| {
            *version == migration.version
                && *success
                && checksum.as_slice() == migration.checksum.as_ref()
        }) {
            return Err(AppError::Unavailable);
        }
    }
    Ok(())
}

/// Seed an initial demo administrator user into the database if not already present.
pub async fn seed(pool: &PgPool) -> Result<crate::modules::users::User, AppError> {
    seed_demo_user(
        pool,
        "demo@example.com",
        "Demo Administrator",
        "Password123!",
        "admin",
    )
    .await
}

/// Seed a user with the specified credentials and role into the database.
pub async fn seed_demo_user(
    pool: &PgPool,
    email: &str,
    display_name: &str,
    password: &str,
    role: &str,
) -> Result<crate::modules::users::User, AppError> {
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version, password_hash::SaltString};
    use rand::rngs::OsRng;
    use uuid::Uuid;

    let normalized_email = email.trim().to_lowercase();

    if let Some(existing) = sqlx::query_as::<_, crate::modules::users::User>(
        "SELECT id, email, display_name, role, is_active, created_at, updated_at \
         FROM users WHERE email = $1",
    )
    .bind(&normalized_email)
    .fetch_optional(pool)
    .await?
    {
        return Ok(existing);
    }

    let parameters = Params::new(19_456, 2, 1, None).map_err(|_| AppError::Internal)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters);
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| AppError::Internal)?
        .to_string();

    let user_id = Uuid::new_v4();
    let user = sqlx::query_as::<_, crate::modules::users::User>(
        "INSERT INTO users (id, email, display_name, password_hash, role, is_active) \
         VALUES ($1, $2, $3, $4, $5, TRUE) \
         RETURNING id, email, display_name, role, is_active, created_at, updated_at",
    )
    .bind(user_id)
    .bind(normalized_email)
    .bind(display_name.trim())
    .bind(password_hash)
    .bind(role)
    .fetch_one(pool)
    .await?;

    Ok(user)
}
