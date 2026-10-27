//! Opt-in tests require TEST_DATABASE_URL with a direct connection to a
//! disposable database. They never read DATABASE_URL or load .env. Each run
//! creates, migrates, and removes only its own randomly named schema.

use std::{collections::HashMap, error::Error, net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Method, Request, StatusCode, header},
};
use enterprise_rust_backend::{
    bootstrap::{AppState, build_app},
    config::{Config, validate_database_url},
    database::MIGRATOR,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::{
    ConnectOptions, PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tower::ServiceExt;
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn Error + Send + Sync>>;

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn auth_and_users_lifecycle_in_an_isolated_schema() -> TestResult {
    let database_url = std::env::var("TEST_DATABASE_URL").map_err(
        |_| "Set TEST_DATABASE_URL to a direct connection to a disposable PostgreSQL database",
    )?;
    validate_database_url(&database_url, false)?;
    let parsed = url::Url::parse(&database_url)
        .map_err(|_| "TEST_DATABASE_URL must be a valid PostgreSQL URL")?;
    ensure(
        !parsed
            .host_str()
            .is_some_and(|host| host.contains("-pooler.")),
        "TEST_DATABASE_URL must use a direct endpoint for migrations and connection-local search_path",
    )?;
    let options: PgConnectOptions = database_url
        .parse()
        .map_err(|_| "TEST_DATABASE_URL must be a valid PostgreSQL connection string")?;
    let options = options
        .application_name("enterprise-rust-backend-tests")
        .disable_statement_logging();
    let control = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options.clone())
        .await?;
    let schema = format!("enterprise_test_{}", Uuid::new_v4().simple());
    let identifier = schema_identifier(&schema)?;
    sqlx::query(&format!("CREATE SCHEMA {identifier}"))
        .execute(&control)
        .await?;

    // Expectations return errors instead of panicking so cleanup also runs
    // when a migration, HTTP request, or assertion fails.
    let result = run_in_schema(options, schema.clone()).await;
    let cleanup = drop_owned_schema(&control, &schema).await;
    control.close().await;
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup_error)) => {
            Err(format!("{error}; schema cleanup also failed: {cleanup_error}").into())
        }
    }
}

async fn run_in_schema(options: PgConnectOptions, schema: String) -> TestResult {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(10))
        .after_connect(move |connection, _| {
            let schema = schema.clone();
            Box::pin(async move {
                sqlx::query("SELECT set_config('search_path', $1, false)")
                    .bind(schema)
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(options)
        .await?;
    let result = async {
        MIGRATOR.run(&pool).await?;
        // The real connection is supplied through the isolated pool; config
        // parsing deliberately does not depend on ambient application secrets.
        let values = HashMap::from([
            ("APP_ENV".into(), "test".into()),
            (
                "DATABASE_URL".into(),
                "postgres://unused:unused@localhost/unused?sslmode=disable".into(),
            ),
            (
                "JWT_SECRET".into(),
                "integration-only-9c3c34b86a7c45fbb8e21fe51f5cab14".into(),
            ),
            ("AUTH_REQUESTS_PER_MINUTE".into(), "1000".into()),
            ("REQUEST_TIMEOUT_SECONDS".into(), "60".into()),
        ]);
        let state = AppState::new(pool.clone(), Config::from_values(&values)?).await?;
        let app = build_app(state);
        exercise_api(&app, &pool).await
    }
    .await;
    pool.close().await;
    result
}

async fn exercise_api(app: &Router, pool: &PgPool) -> TestResult {
    let password = "integration correct horse battery staple";
    registration_is_atomic(app, pool, password).await?;
    let registered = request(
        app,
        Method::POST,
        "/api/v1/auth/register",
        None,
        Some(json!({
            "email": "MEMBER@Example.test", "display_name": "  Test Member  ", "password": password,
        })),
        StatusCode::CREATED,
    )
    .await?;
    ensure(
        registered["data"]["user"]["email"] == "member@example.test",
        "Registration must normalize email",
    )?;
    ensure(
        registered["data"]["user"]["display_name"] == "Test Member",
        "Registration must trim display names",
    )?;
    ensure(
        registered["data"]["user"]["role"] == "user",
        "Registration must create an ordinary user",
    )?;
    ensure(
        registered["data"]["user"].get("password_hash").is_none(),
        "Public users must not contain password hashes",
    )?;
    let user_id: Uuid = string_at(&registered, "/data/user/id")?.parse()?;
    let initial_access = string_at(&registered, "/data/access_token")?;
    let initial_refresh = string_at(&registered, "/data/refresh_token")?;

    request(
        app,
        Method::GET,
        "/api/v1/users/me",
        None,
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/register",
        None,
        Some(json!({
            "email": "member@example.test", "display_name": "Duplicate", "password": password,
        })),
        StatusCode::CONFLICT,
    )
    .await?;
    request(app, Method::POST, "/api/v1/auth/register", None, Some(json!({
        "email": "admin-attempt@example.test", "display_name": "Attempt", "password": password, "role": "admin",
    })), StatusCode::UNPROCESSABLE_ENTITY).await?;

    let wrong_password = request(
        app,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({
            "email": "member@example.test", "password": "wrong but long password",
        })),
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    let missing_account = request(
        app,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({
            "email": "missing@example.test", "password": "wrong but long password",
        })),
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    ensure(
        wrong_password == missing_account,
        "Invalid credentials must not disclose whether the account exists",
    )?;

    let logged_in = login(app, password).await?;
    let access = string_at(&logged_in, "/data/access_token")?;
    let refresh = string_at(&logged_in, "/data/refresh_token")?;
    let profile = request(
        app,
        Method::GET,
        "/api/v1/users/me",
        Some(access),
        None,
        StatusCode::OK,
    )
    .await?;
    ensure(
        profile["data"]["id"] == user_id.to_string(),
        "Authenticated profile must belong to the caller",
    )?;
    let updated = request(
        app,
        Method::PATCH,
        "/api/v1/users/me",
        Some(access),
        Some(json!({
            "display_name": "  Updated Member  ",
        })),
        StatusCode::OK,
    )
    .await?;
    ensure(
        updated["data"]["display_name"] == "Updated Member",
        "Profile update must trim the display name",
    )?;
    ensure(
        updated["data"]["role"] == "user",
        "Profile updates must preserve role",
    )?;
    request(
        app,
        Method::PATCH,
        "/api/v1/users/me",
        Some(access),
        Some(json!({
            "display_name": "   ",
        })),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await?;
    request(
        app,
        Method::PATCH,
        "/api/v1/users/me",
        Some(access),
        Some(json!({
            "display_name": "Attempt", "role": "admin",
        })),
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await?;
    request(
        app,
        Method::GET,
        "/api/v1/users/",
        Some(access),
        None,
        StatusCode::FORBIDDEN,
    )
    .await?;
    let by_id = format!("/api/v1/users/{user_id}");
    request(
        app,
        Method::GET,
        &by_id,
        Some(access),
        None,
        StatusCode::FORBIDDEN,
    )
    .await?;

    // Current database roles must govern access even for previously issued JWTs.
    sqlx::query("UPDATE users SET role = 'admin' WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    let listed = request(
        app,
        Method::GET,
        "/api/v1/users/?page=1&per_page=1",
        Some(access),
        None,
        StatusCode::OK,
    )
    .await?;
    ensure(
        listed["data"]["total"] == 1,
        "Listing total must include exactly the registered account",
    )?;
    ensure(
        listed["data"]["items"][0]["id"] == user_id.to_string(),
        "Listing must return the registered account",
    )?;
    request(app, Method::GET, &by_id, Some(access), None, StatusCode::OK).await?;
    let empty_page = request(
        app,
        Method::GET,
        "/api/v1/users/?page=2&per_page=1",
        Some(access),
        None,
        StatusCode::OK,
    )
    .await?;
    ensure(
        empty_page["data"]["total"] == 1 && empty_page["data"]["items"] == json!([]),
        "Empty pages must retain the correct total",
    )?;
    request(
        app,
        Method::GET,
        "/api/v1/users/?page=0",
        Some(access),
        None,
        StatusCode::UNPROCESSABLE_ENTITY,
    )
    .await?;
    sqlx::query("UPDATE users SET role = 'user' WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    request(
        app,
        Method::GET,
        "/api/v1/users/",
        Some(access),
        None,
        StatusCode::FORBIDDEN,
    )
    .await?;

    let rotated = request(
        app,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({
            "refresh_token": refresh,
        })),
        StatusCode::OK,
    )
    .await?;
    let rotated_access = string_at(&rotated, "/data/access_token")?;
    let rotated_refresh = string_at(&rotated, "/data/refresh_token")?;
    ensure(rotated_refresh != refresh, "Refresh must rotate the token")?;
    request(
        app,
        Method::GET,
        "/api/v1/users/me",
        Some(rotated_access),
        None,
        StatusCode::OK,
    )
    .await?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({
            "refresh_token": refresh,
        })),
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({
            "refresh_token": rotated_refresh,
        })),
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::GET,
        "/api/v1/users/me",
        Some(rotated_access),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::GET,
        "/api/v1/users/me",
        Some(access),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::GET,
        "/api/v1/users/me",
        Some(initial_access),
        None,
        StatusCode::OK,
    )
    .await?;

    let logout_session = login(app, password).await?;
    let logout_access = string_at(&logout_session, "/data/access_token")?;
    let logout_refresh = string_at(&logout_session, "/data/refresh_token")?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/logout",
        Some(logout_access),
        None,
        StatusCode::NO_CONTENT,
    )
    .await?;
    request(
        app,
        Method::GET,
        "/api/v1/users/me",
        Some(logout_access),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({
            "refresh_token": logout_refresh,
        })),
        StatusCode::UNAUTHORIZED,
    )
    .await?;

    sqlx::query("UPDATE users SET is_active = FALSE WHERE id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    request(
        app,
        Method::GET,
        "/api/v1/users/me",
        Some(initial_access),
        None,
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({
            "email": "member@example.test", "password": password,
        })),
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/refresh",
        None,
        Some(json!({
            "refresh_token": initial_refresh,
        })),
        StatusCode::UNAUTHORIZED,
    )
    .await?;
    Ok(())
}

async fn registration_is_atomic(app: &Router, pool: &PgPool, password: &str) -> TestResult {
    // Reject the downstream session insert after the account insert succeeds.
    // Both writes must roll back together.
    sqlx::query(
        "CREATE FUNCTION reject_test_session() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'intentional integration test failure'; END; $$",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TRIGGER reject_test_session_insert BEFORE INSERT ON auth_sessions
         FOR EACH ROW EXECUTE FUNCTION reject_test_session()",
    )
    .execute(pool)
    .await?;
    request(
        app,
        Method::POST,
        "/api/v1/auth/register",
        None,
        Some(json!({
            "email": "rollback@example.test", "display_name": "Rollback", "password": password,
        })),
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await?;
    let accounts = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM users WHERE email = 'rollback@example.test'",
    )
    .fetch_one(pool)
    .await?;
    ensure(
        accounts == 0,
        "Failed session creation must not leave an account behind",
    )?;
    sqlx::query("DROP TRIGGER reject_test_session_insert ON auth_sessions")
        .execute(pool)
        .await?;
    sqlx::query("DROP FUNCTION reject_test_session()")
        .execute(pool)
        .await?;
    Ok(())
}

async fn login(app: &Router, password: &str) -> TestResult<Value> {
    request(
        app,
        Method::POST,
        "/api/v1/auth/login",
        None,
        Some(json!({
            "email": "Member@EXAMPLE.test", "password": password,
        })),
        StatusCode::OK,
    )
    .await
}

async fn request(
    app: &Router,
    method: Method,
    uri: &str,
    access_token: Option<&str>,
    payload: Option<Value>,
    expected_status: StatusCode,
) -> TestResult<Value> {
    let mut builder = Request::builder().method(method.clone()).uri(uri);
    if let Some(token) = access_token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match payload {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value)?)
        }
        None => Body::empty(),
    };
    let mut request = builder.body(body)?;
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    // Do not print response bodies: successful auth responses contain secrets.
    ensure(
        status == expected_status,
        &format!("{method} {uri}: expected {expected_status}, received {status}"),
    )?;
    let body = response.into_body().collect().await?.to_bytes();
    if body.is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_slice(&body)?)
    }
}

fn string_at<'a>(value: &'a Value, pointer: &str) -> TestResult<&'a str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Expected string response field {pointer}").into())
}

fn ensure(condition: bool, message: &str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(message.to_owned().into())
    }
}

fn schema_identifier(schema: &str) -> TestResult<String> {
    let suffix = schema
        .strip_prefix("enterprise_test_")
        .ok_or("Refusing to operate on a schema outside the test namespace")?;
    ensure(
        suffix.len() == 32 && suffix.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Refusing to operate on an invalid test schema identifier",
    )?;
    Ok(format!("\"{schema}\""))
}

async fn drop_owned_schema(pool: &PgPool, schema: &str) -> TestResult {
    let identifier = schema_identifier(schema)?;
    sqlx::query(&format!("DROP SCHEMA {identifier} CASCADE"))
        .execute(pool)
        .await?;
    Ok(())
}

#[test]
fn cleanup_guard_rejects_other_schemas_and_interpolated_sql() {
    for unsafe_name in [
        "public",
        "enterprise_test_",
        "enterprise_test_123; DROP SCHEMA public CASCADE",
        "other_00000000000000000000000000000000",
    ] {
        assert!(schema_identifier(unsafe_name).is_err());
    }
    assert!(schema_identifier("enterprise_test_00112233445566778899aabbccddeeff").is_ok());
}

#[tokio::test]
async fn demo_test_case_verifies_password_hashing_and_user_creation() -> TestResult {
    use argon2::{
        Algorithm, Argon2, Params, PasswordHasher, PasswordVerifier, Version,
        password_hash::{PasswordHash, SaltString},
    };
    use rand::rngs::OsRng;

    let password = "Password123!";
    let parameters = Params::new(19_456, 2, 1, None).map_err(|e| e.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters);
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| e.to_string())?
        .to_string();

    let parsed_hash = PasswordHash::new(&hash).map_err(|e| e.to_string())?;
    ensure(
        argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok(),
        "Argon2 hash verification must succeed for valid demo password",
    )?;
    ensure(
        argon2
            .verify_password(b"wrong_password", &parsed_hash)
            .is_err(),
        "Argon2 hash verification must reject incorrect password",
    )?;

    // Verify demo user values satisfy validation constraints
    let demo_email = "demo@example.com";
    let demo_display_name = "Demo Administrator";
    ensure(
        demo_email.contains('@') && demo_email == demo_email.to_lowercase(),
        "Demo email must be normalized lowercase",
    )?;
    ensure(
        !demo_display_name.trim().is_empty() && demo_display_name.len() <= 100,
        "Demo display name must be non-empty and bounded",
    )?;
    Ok(())
}
