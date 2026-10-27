use sqlx::{PgExecutor, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::shared::{error::AppError, pagination::Pagination};

use super::model::{NewUser, User, UserCredentials};

#[derive(Clone)]
pub(super) struct UsersRepository {
    pool: PgPool,
}

impl UsersRepository {
    pub(super) fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(super) async fn create(&self, input: NewUser) -> Result<User, AppError> {
        Self::insert(&self.pool, input).await
    }

    pub(super) async fn create_in_transaction(
        &self,
        input: NewUser,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<User, AppError> {
        Self::insert(&mut **transaction, input).await
    }

    async fn insert<'e>(executor: impl PgExecutor<'e>, input: NewUser) -> Result<User, AppError> {
        sqlx::query_as::<_, User>(
            "INSERT INTO users (id, email, display_name, password_hash)
             VALUES ($1, $2, $3, $4)
             RETURNING id, email, display_name, role, is_active, created_at, updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(input.email)
        .bind(input.display_name)
        .bind(input.password_hash)
        .fetch_one(executor)
        .await
        .map_err(|error| {
            if let sqlx::Error::Database(ref database_error) = error
                && database_error.is_unique_violation()
                && database_error.constraint() == Some("users_email_key")
            {
                return AppError::Conflict("An account with this email already exists");
            }
            error.into()
        })
    }

    pub(super) async fn credentials_by_email(
        &self,
        email: &str,
    ) -> Result<Option<UserCredentials>, AppError> {
        Ok(sqlx::query_as::<_, UserCredentials>(
            "SELECT id, email, display_name, role, is_active, created_at, updated_at,
                    password_hash
             FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub(super) async fn get_by_id(&self, id: Uuid) -> Result<Option<User>, AppError> {
        Ok(sqlx::query_as::<_, User>(
            "SELECT id, email, display_name, role, is_active, created_at, updated_at
             FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub(super) async fn update_profile(
        &self,
        id: Uuid,
        display_name: &str,
    ) -> Result<Option<User>, AppError> {
        Ok(sqlx::query_as::<_, User>(
            "UPDATE users SET display_name = $2
             WHERE id = $1 AND is_active = TRUE
             RETURNING id, email, display_name, role, is_active, created_at, updated_at",
        )
        .bind(id)
        .bind(display_name)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub(super) async fn list(&self, pagination: &Pagination) -> Result<(Vec<User>, i64), AppError> {
        // A consistent snapshot prevents the total and page from disagreeing if
        // registrations happen between the two reads.
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *transaction)
            .await?;

        let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&mut *transaction)
            .await?;

        let items = sqlx::query_as::<_, User>(
            "SELECT id, email, display_name, role, is_active, created_at, updated_at
             FROM users ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
        )
        .bind(pagination.limit())
        .bind(pagination.offset())
        .fetch_all(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok((items, total))
    }
}
