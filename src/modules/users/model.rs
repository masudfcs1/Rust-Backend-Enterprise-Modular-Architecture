use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use validator::Validate;

use super::dto::validate_display_name;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(type_name = "text", rename_all = "lowercase")]
pub enum Role {
    User,
    Admin,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Admin => "admin",
        }
    }
}

/// Public profile data. Credentials deliberately live in a separate type.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    pub role: Role,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Trusted service input; callers cannot choose a role or account status.
#[derive(Validate)]
pub struct NewUser {
    #[validate(email, length(max = 254))]
    pub email: String,
    #[validate(custom(function = "validate_display_name"))]
    pub display_name: String,
    pub password_hash: String,
}

/// Authentication-only data. Do not implement serialization or Debug on this type.
#[derive(sqlx::FromRow)]
pub struct UserCredentials {
    #[sqlx(flatten)]
    pub user: User,
    pub password_hash: String,
}
