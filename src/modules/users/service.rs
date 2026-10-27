use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use validator::Validate;

use crate::shared::{
    error::AppError,
    pagination::{Paginated, Pagination},
};

use super::{
    dto::UpdateProfileRequest,
    model::{NewUser, User, UserCredentials},
    repository::UsersRepository,
};

#[derive(Clone)]
pub struct UsersService {
    repository: UsersRepository,
}

impl UsersService {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repository: UsersRepository::new(pool),
        }
    }

    pub async fn create(&self, input: NewUser) -> Result<User, AppError> {
        self.repository.create(validate_new_user(input)?).await
    }

    /// Participates in an owning workflow's transaction without committing it.
    /// Authentication uses this to create an account and its session atomically.
    pub async fn create_in_transaction(
        &self,
        input: NewUser,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<User, AppError> {
        self.repository
            .create_in_transaction(validate_new_user(input)?, transaction)
            .await
    }

    pub async fn credentials_by_email(
        &self,
        email: &str,
    ) -> Result<Option<UserCredentials>, AppError> {
        self.repository
            .credentials_by_email(&normalize_email(email))
            .await
    }

    pub async fn get_by_id(&self, id: Uuid) -> Result<User, AppError> {
        self.repository
            .get_by_id(id)
            .await?
            .ok_or(AppError::NotFound("User"))
    }

    pub(super) async fn update_profile(
        &self,
        id: Uuid,
        input: UpdateProfileRequest,
    ) -> Result<User, AppError> {
        input
            .validate()
            .map_err(|_| AppError::Validation("Invalid display name".to_owned()))?;
        self.repository
            .update_profile(id, input.display_name.trim())
            .await?
            .ok_or(AppError::NotFound("User"))
    }

    pub(super) async fn list(&self, pagination: Pagination) -> Result<Paginated<User>, AppError> {
        pagination.validate()?;
        let (items, total) = self.repository.list(&pagination).await?;
        Ok(Paginated {
            items,
            page: pagination.page,
            per_page: pagination.per_page,
            total,
        })
    }
}

pub(super) fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

fn validate_new_user(input: NewUser) -> Result<NewUser, AppError> {
    let input = NewUser {
        email: normalize_email(&input.email),
        display_name: input.display_name.trim().to_owned(),
        password_hash: input.password_hash,
    };
    input
        .validate()
        .map_err(|_| AppError::Validation("Invalid email or display name".to_owned()))?;

    // A missing hash is a programming error in a trusted caller, not input
    // the public profile API accepts.
    if input.password_hash.is_empty() || input.password_hash.len() > 1024 {
        return Err(AppError::Internal);
    }
    Ok(input)
}
