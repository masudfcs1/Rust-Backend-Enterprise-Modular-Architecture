use serde::Deserialize;
use validator::{Validate, ValidationError};

/// Profile updates exclude identity, role, password, and activation status.
#[derive(Debug, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct UpdateProfileRequest {
    #[validate(custom(function = "validate_display_name"))]
    pub display_name: String,
}

pub(super) fn validate_display_name(value: &str) -> Result<(), ValidationError> {
    let name = value.trim();
    if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
        return Err(ValidationError::new("display_name").with_message(
            "Display name must contain 1 to 100 characters without control characters".into(),
        ));
    }
    Ok(())
}
