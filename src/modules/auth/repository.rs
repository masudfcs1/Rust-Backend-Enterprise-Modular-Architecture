use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::shared::AppError;

#[derive(Clone)]
pub(super) struct SessionRepository {
    pool: PgPool,
}

#[derive(FromRow)]
pub(super) struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl SessionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn begin(&self) -> Result<Transaction<'static, Postgres>, AppError> {
        Ok(self.pool.begin().await?)
    }

    pub async fn create_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        id: Uuid,
        user_id: Uuid,
        expires_at: DateTime<Utc>,
        refresh_hash: Vec<u8>,
    ) -> Result<(), AppError> {
        sqlx::query("INSERT INTO auth_sessions (id, user_id, expires_at) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(user_id)
            .bind(expires_at)
            .execute(&mut **transaction)
            .await?;
        sqlx::query("INSERT INTO auth_refresh_tokens (token_hash, session_id) VALUES ($1, $2)")
            .bind(refresh_hash)
            .bind(id)
            .execute(&mut **transaction)
            .await?;
        Ok(())
    }

    pub async fn is_active(&self, session_id: Uuid, user_id: Uuid) -> Result<bool, AppError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM auth_sessions WHERE id = $1 AND user_id = $2 \
             AND revoked_at IS NULL AND expires_at > NOW())",
        )
        .bind(session_id)
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn revoke(&self, session_id: Uuid, user_id: Uuid) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE auth_sessions SET revoked_at = COALESCE(revoked_at, NOW()) \
             WHERE id = $1 AND user_id = $2",
        )
        .bind(session_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn rotate(
        &self,
        previous_hash: Vec<u8>,
        replacement_hash: Vec<u8>,
    ) -> Result<Session, AppError> {
        let mut transaction = self.pool.begin().await?;
        let session_id = sqlx::query_scalar::<_, Uuid>(
            "SELECT session_id FROM auth_refresh_tokens WHERE token_hash = $1",
        )
        .bind(&previous_hash)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(AppError::Unauthorized)?;

        // Serialize all rotations and revocations for this token family.
        let session = sqlx::query_as::<_, Session>(
            "SELECT id, user_id, expires_at, revoked_at FROM auth_sessions \
             WHERE id = $1 FOR UPDATE",
        )
        .bind(session_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(AppError::Unauthorized)?;
        if session.revoked_at.is_some() || session.expires_at <= Utc::now() {
            return Err(AppError::Unauthorized);
        }

        // Read consumption AFTER acquiring the session lock. A concurrent rotation may
        // have committed while this request waited; a joined pre-lock snapshot is unsafe.
        let consumed = sqlx::query_scalar::<_, bool>(
            "SELECT consumed_at IS NOT NULL FROM auth_refresh_tokens WHERE token_hash = $1",
        )
        .bind(&previous_hash)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(AppError::Unauthorized)?;
        if consumed {
            sqlx::query("UPDATE auth_sessions SET revoked_at = NOW() WHERE id = $1")
                .bind(session.id)
                .execute(&mut *transaction)
                .await?;
            // Commit the revocation before returning the authentication failure.
            transaction.commit().await?;
            return Err(AppError::Unauthorized);
        }

        sqlx::query("UPDATE auth_refresh_tokens SET consumed_at = NOW() WHERE token_hash = $1")
            .bind(previous_hash)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("INSERT INTO auth_refresh_tokens (token_hash, session_id) VALUES ($1, $2)")
            .bind(replacement_hash)
            .bind(session.id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE auth_sessions SET last_rotated_at = NOW() WHERE id = $1")
            .bind(session.id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(session)
    }
}
