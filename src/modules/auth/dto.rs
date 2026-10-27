use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

use crate::modules::users::User;

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    #[validate(email, length(max = 254))]
    pub email: String,
    #[validate(length(min = 1, max = 100))]
    pub display_name: String,
    #[validate(custom(function = "validate_registration_password"))]
    pub password: String,
}

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    #[validate(email, length(max = 254))]
    pub email: String,
    #[validate(custom(function = "validate_login_password"))]
    pub password: String,
}

#[derive(Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {
    #[validate(custom(function = "validate_refresh_token"))]
    pub refresh_token: String,
}

// Deliberately not Debug: responses contain bearer credentials.
#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub user: User,
}

fn validate_registration_password(value: &str) -> Result<(), ValidationError> {
    if value.chars().count() < 12 || value.len() > 128 {
        return Err(ValidationError::new(
            "password_requires_12_characters_and_at_most_128_bytes",
        ));
    }
    Ok(())
}

fn validate_login_password(value: &str) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > 128 {
        return Err(ValidationError::new("password_requires_1_to_128_bytes"));
    }
    Ok(())
}

pub(super) fn validate_refresh_token(value: &str) -> Result<(), ValidationError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ValidationError::new("invalid_refresh_token"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_rejects_short_and_excessive_byte_length_passwords() {
        assert!(validate_registration_password("short").is_err());
        assert!(validate_registration_password(&"x".repeat(129)).is_err());
        assert!(validate_registration_password(&"🔐".repeat(33)).is_err());
        assert!(validate_registration_password("a long passphrase is good").is_ok());
    }

    #[test]
    fn registration_cannot_supply_a_role() {
        let body = r#"{"email":"member@example.com","display_name":"Member","password":"a long password","role":"admin"}"#;
        assert!(serde_json::from_str::<RegisterRequest>(body).is_err());
    }
}
