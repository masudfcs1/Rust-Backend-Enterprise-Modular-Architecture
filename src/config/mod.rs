use std::{collections::HashMap, net::IpAddr, str::FromStr};

use axum::http::HeaderValue;
use thiserror::Error;
use url::Url;

/// Secrets deliberately have no Debug or Serialize implementation.
#[derive(Clone)]
pub struct Config {
    pub environment: String,
    pub host: IpAddr,
    pub port: u16,
    pub database_url: String,
    pub database_url_unpooled: Option<String>,
    pub database_max_connections: u32,
    pub database_acquire_timeout_seconds: u64,
    pub auth: AuthConfig,
    pub cors_allowed_origins: Vec<HeaderValue>,
    pub request_timeout_seconds: u64,
    pub request_body_limit_bytes: usize,
    pub auth_requests_per_minute: u32,
}

#[derive(Clone)]
pub struct AuthConfig {
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub jwt_audience: String,
    pub access_ttl_seconds: u64,
    pub refresh_ttl_seconds: u64,
    pub password_concurrency: usize,
}

#[derive(Debug, Error)]
#[error("configuration error: {0}")]
pub struct ConfigError(pub String);

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_values(&std::env::vars().collect())
    }

    /// Pure parsing makes configuration tests independent of process-global env.
    pub fn from_values(values: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let get =
            |key: &str, default: &str| values.get(key).cloned().unwrap_or_else(|| default.into());
        let required = |key: &str| -> Result<String, ConfigError> {
            values
                .get(key)
                .filter(|v| !v.trim().is_empty())
                .cloned()
                .ok_or_else(|| ConfigError(format!("{key} is required")))
        };
        let environment = get("APP_ENV", "development");
        if !matches!(environment.as_str(), "development" | "test" | "production") {
            return Err(ConfigError(
                "APP_ENV must be development, test, or production".into(),
            ));
        }
        let database_url = required("DATABASE_URL")?;
        validate_database_url(&database_url, environment == "production")?;
        let database_url_unpooled = values
            .get("DATABASE_URL_UNPOOLED")
            .filter(|s| !s.is_empty())
            .cloned();
        if let Some(url) = &database_url_unpooled {
            validate_database_url(url, environment == "production")?;
        }
        let jwt_secret = required("JWT_SECRET")?;
        if jwt_secret.len() < 32 || jwt_secret.to_ascii_lowercase().contains("replace-with") {
            return Err(ConfigError(
                "JWT_SECRET must contain at least 32 bytes of randomly generated secret material"
                    .into(),
            ));
        }
        let jwt_issuer = get("JWT_ISSUER", "enterprise-rust-backend");
        let jwt_audience = get("JWT_AUDIENCE", "enterprise-api");
        if jwt_issuer.trim().is_empty() || jwt_audience.trim().is_empty() {
            return Err(ConfigError(
                "JWT_ISSUER and JWT_AUDIENCE cannot be blank".into(),
            ));
        }
        let access_ttl_seconds = number(values, "ACCESS_TOKEN_TTL_SECONDS", 900u64, 60, 3600)?;
        let refresh_ttl_seconds = number(
            values,
            "REFRESH_TOKEN_TTL_SECONDS",
            604800u64,
            3600,
            2592000,
        )?;
        if refresh_ttl_seconds <= access_ttl_seconds {
            return Err(ConfigError(
                "refresh token lifetime must exceed access token lifetime".into(),
            ));
        }
        let mut cors_allowed_origins = Vec::new();
        for origin in get("CORS_ALLOWED_ORIGINS", "")
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let parsed = Url::parse(origin).map_err(|_| {
                ConfigError("CORS_ALLOWED_ORIGINS contains an invalid origin".into())
            })?;
            if !matches!(parsed.scheme(), "http" | "https")
                || parsed.host_str().is_none()
                || parsed.path() != "/"
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || (environment == "production" && parsed.scheme() != "https")
            {
                return Err(ConfigError(
                    "CORS origins must be exact HTTP(S) origins, with HTTPS in production".into(),
                ));
            }
            cors_allowed_origins.push(
                HeaderValue::from_str(&parsed.origin().ascii_serialization())
                    .map_err(|_| ConfigError("invalid CORS header value".into()))?,
            );
        }
        Ok(Self {
            environment,
            host: get("HTTP_HOST", "127.0.0.1")
                .parse()
                .map_err(|_| ConfigError("HTTP_HOST must be an IP address".into()))?,
            port: number(values, "HTTP_PORT", 3000u16, 1, 65535)?,
            database_url,
            database_url_unpooled,
            database_max_connections: number(values, "DATABASE_MAX_CONNECTIONS", 10u32, 1, 100)?,
            database_acquire_timeout_seconds: number(
                values,
                "DATABASE_ACQUIRE_TIMEOUT_SECONDS",
                10u64,
                1,
                60,
            )?,
            auth: AuthConfig {
                jwt_secret,
                jwt_issuer,
                jwt_audience,
                access_ttl_seconds,
                refresh_ttl_seconds,
                password_concurrency: number(values, "PASSWORD_CONCURRENCY", 4usize, 1, 32)?,
            },
            cors_allowed_origins,
            request_timeout_seconds: number(values, "REQUEST_TIMEOUT_SECONDS", 15u64, 1, 120)?,
            request_body_limit_bytes: number(
                values,
                "REQUEST_BODY_LIMIT_BYTES",
                16384usize,
                1024,
                1048576,
            )?,
            auth_requests_per_minute: number(values, "AUTH_REQUESTS_PER_MINUTE", 20u32, 1, 10000)?,
        })
    }
}

fn number<T>(
    values: &HashMap<String, String>,
    key: &str,
    default: T,
    min: T,
    max: T,
) -> Result<T, ConfigError>
where
    T: FromStr + PartialOrd,
{
    let value = match values.get(key) {
        Some(raw) => raw
            .parse()
            .map_err(|_| ConfigError(format!("{key} must be a valid number")))?,
        None => default,
    };
    if value < min || value > max {
        return Err(ConfigError(format!("{key} is outside the supported range")));
    }
    Ok(value)
}

pub fn validate_database_url(raw: &str, production: bool) -> Result<(), ConfigError> {
    let url = Url::parse(raw)
        .map_err(|_| ConfigError("DATABASE_URL is not a valid PostgreSQL URL".into()))?;
    if !matches!(url.scheme(), "postgres" | "postgresql")
        || url.host_str().is_none()
        || url.path().len() < 2
    {
        return Err(ConfigError(
            "DATABASE_URL must specify a PostgreSQL host and database".into(),
        ));
    }
    // SQLx otherwise silently ignores unknown query parameters (including channel_binding).
    let mut seen = std::collections::HashSet::new();
    for (key, value) in url.query_pairs() {
        if !seen.insert(key.clone()) {
            return Err(ConfigError(
                "DATABASE_URL contains duplicate options".into(),
            ));
        }
        match key.as_ref() {
            "sslmode" if matches!(value.as_ref(), "disable" | "require" | "verify-ca" | "verify-full") => {},
            "sslrootcert" | "application_name" | "statement-cache-capacity" => {},
            "channel_binding" => return Err(ConfigError("SQLx does not implement channel binding; use sslmode=verify-full without channel_binding, or a driver that enforces channel binding".into())),
            _ => return Err(ConfigError("DATABASE_URL contains an unsupported connection option".into())),
        }
    }
    let is_local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "::1" | "[::1]" | "db")
    );
    if (production || !is_local)
        && !url
            .query_pairs()
            .any(|(k, v)| k == "sslmode" && v == "verify-full")
    {
        return Err(ConfigError(
            "remote/production PostgreSQL requires sslmode=verify-full".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn values() -> HashMap<String, String> {
        [
            (
                "DATABASE_URL",
                "postgres://app:secret@localhost/test?sslmode=disable",
            ),
            ("JWT_SECRET", "test-key-7caff02c47b9402789028ec675699bbb"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }
    #[test]
    fn rejects_weak_secrets_and_invalid_limits_without_echoing_values() {
        let mut v = values();
        v.insert("JWT_SECRET".into(), "do-not-print-me".into());
        let err = Config::from_values(&v).err().unwrap().to_string();
        assert!(!err.contains("do-not-print-me"));
        let mut v = values();
        v.insert("DATABASE_MAX_CONNECTIONS".into(), "0".into());
        assert!(Config::from_values(&v).is_err());
    }
    #[test]
    fn requires_verified_tls_for_remote_databases_and_rejects_ignored_security_options() {
        assert!(
            validate_database_url("postgres://u:p@remote.example/db?sslmode=require", false)
                .is_err()
        );
        assert!(
            validate_database_url(
                "postgres://u:p@remote.example/db?sslmode=verify-full",
                false
            )
            .is_ok()
        );
        assert!(
            validate_database_url(
                "postgres://u:p@remote.example/db?sslmode=verify-full&channel_binding=require",
                false
            )
            .is_err()
        );
        assert!(
            validate_database_url(
                "postgres://u:p@remote.example/db?sslmode=verify-full&sslmode=disable",
                false
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_wildcard_or_path_cors_and_normalizes_origin() {
        let mut v = values();
        v.insert("CORS_ALLOWED_ORIGINS".into(), "*".into());
        assert!(Config::from_values(&v).is_err());
        v.insert(
            "CORS_ALLOWED_ORIGINS".into(),
            "https://example.com/path".into(),
        );
        assert!(Config::from_values(&v).is_err());
        v.insert("CORS_ALLOWED_ORIGINS".into(), "https://example.com/".into());
        assert_eq!(
            Config::from_values(&v).unwrap().cors_allowed_origins[0],
            "https://example.com"
        );
    }
}
