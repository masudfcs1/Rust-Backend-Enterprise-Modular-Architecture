use chrono::Utc;
use uuid::Uuid;
use validator::Validate;

use super::{
    NewUser, Role, User,
    dto::{UpdateProfileRequest, validate_display_name},
    handler::require_admin,
    service::normalize_email,
};

#[test]
fn profile_updates_reject_privilege_and_identity_fields() {
    for field in ["role", "is_active", "password_hash", "email", "id"] {
        let value = serde_json::json!({"display_name": "Ada", field: "admin"});
        assert!(serde_json::from_value::<UpdateProfileRequest>(value).is_err());
    }
}

#[test]
fn display_names_accept_unicode_and_reject_blank_control_or_oversized_values() {
    for valid in ["Ada Lovelace", "  Masud Rana  ", "মাসুদ রানা", "李明"] {
        assert!(validate_display_name(valid).is_ok(), "{valid:?}");
    }
    for invalid in ["", "   ", "Ada\nLovelace", "A\u{0000}B"] {
        assert!(validate_display_name(invalid).is_err(), "{invalid:?}");
    }
    assert!(validate_display_name(&"名".repeat(100)).is_ok());
    assert!(validate_display_name(&"名".repeat(101)).is_err());
}

#[test]
fn registration_identity_requires_a_valid_email_and_name() {
    let invalid_email = NewUser {
        email: "invalid-address".into(),
        display_name: "Ada".into(),
        password_hash: "opaque-hash".into(),
    };
    assert!(invalid_email.validate().is_err());

    let invalid_name = NewUser {
        email: "ada@example.com".into(),
        display_name: "  ".into(),
        password_hash: "opaque-hash".into(),
    };
    assert!(invalid_name.validate().is_err());
}

#[test]
fn identity_lookup_uses_the_same_case_insensitive_normalization() {
    assert_eq!(normalize_email("  ADA@Example.COM \n"), "ada@example.com");
}

#[test]
fn user_listing_is_limited_to_administrators() {
    assert!(require_admin(Role::Admin).is_ok());
    assert!(matches!(
        require_admin(Role::User),
        Err(crate::shared::error::AppError::Forbidden)
    ));
}

#[test]
fn public_profiles_have_no_credentials() {
    let now = Utc::now();
    let user = User {
        id: Uuid::new_v4(),
        email: "ada@example.com".into(),
        display_name: "Ada".into(),
        role: Role::User,
        is_active: true,
        created_at: now,
        updated_at: now,
    };
    let value = serde_json::to_value(user).unwrap();
    assert_eq!(value["role"], "user");
    assert!(value.get("password_hash").is_none());
    assert!(value.get("password").is_none());
}

#[test]
fn roles_are_closed_and_use_lowercase_api_values() {
    assert_eq!(serde_json::to_string(&Role::Admin).unwrap(), "\"admin\"");
    assert!(serde_json::from_str::<Role>("\"superadmin\"").is_err());
    assert!(serde_json::from_str::<Role>("\"ADMIN\"").is_err());
}
