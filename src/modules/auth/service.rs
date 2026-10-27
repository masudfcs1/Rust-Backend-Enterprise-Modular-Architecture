use chrono::{Duration, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;
use validator::Validate;

use crate::{
    config::AuthConfig,
    modules::users::{NewUser, Role, User, UsersService},
    shared::AppError,
};

use super::{
    dto::{LoginRequest, RegisterRequest, TokenResponse, validate_refresh_token},
    password::PasswordService,
    repository::SessionRepository,
    token::{TokenCodec, new_refresh_token, refresh_token_hash},
};

#[derive(Clone)]
pub struct AuthService {
    sessions: SessionRepository,
    users: UsersService,
    passwords: PasswordService,
    tokens: TokenCodec,
    dummy_password_hash: String,
    refresh_ttl_seconds: i64,
}

#[derive(Clone, Debug)]
pub struct Principal {
    pub user_id: Uuid,
    pub role: Role,
    pub session_id: Uuid,
}

impl AuthService {
    pub async fn new(
        pool: PgPool,
        users: UsersService,
        config: AuthConfig,
    ) -> Result<Self, AppError> {
        if config.refresh_ttl_seconds < config.access_ttl_seconds {
            return Err(AppError::Internal);
        }
        let refresh_ttl_seconds =
            i64::try_from(config.refresh_ttl_seconds).map_err(|_| AppError::Internal)?;
        let tokens = TokenCodec::new(&config)?;
        let passwords = PasswordService::new(config.password_concurrency)?;
        // A missing account still performs a full Argon2 verification, avoiding a
        // cheap username-enumeration path. The random password is never retained.
        let dummy_password_hash = passwords.hash(new_refresh_token()).await?;
        Ok(Self {
            sessions: SessionRepository::new(pool),
            users,
            passwords,
            tokens,
            dummy_password_hash,
            refresh_ttl_seconds,
        })
    }

    pub async fn register(&self, input: RegisterRequest) -> Result<TokenResponse, AppError> {
        input
            .validate()
            .map_err(|_| AppError::Validation("Invalid registration details".to_owned()))?;
        let password_hash = self.passwords.hash(input.password).await?;
        // Hashing consumes no connection. The account and initial session then
        // commit together, so failed issuance cannot leave a partial registration.
        let mut transaction = self.sessions.begin().await?;
        let user = self
            .users
            .create_in_transaction(
                NewUser {
                    email: input.email,
                    display_name: input.display_name,
                    password_hash,
                },
                &mut transaction,
            )
            .await?;
        let response = self
            .create_session_in_transaction(user, &mut transaction)
            .await?;
        transaction.commit().await?;
        Ok(response)
    }

    pub async fn login(&self, input: LoginRequest) -> Result<TokenResponse, AppError> {
        input
            .validate()
            .map_err(|_| AppError::Validation("Invalid login details".to_owned()))?;
        let credentials = self.users.credentials_by_email(&input.email).await?;
        let hash = credentials
            .as_ref()
            .map(|credentials| credentials.password_hash.clone())
            .unwrap_or_else(|| self.dummy_password_hash.clone());
        let valid_password = self.passwords.verify(input.password, hash).await?;
        let credentials = credentials.ok_or(AppError::Unauthorized)?;
        if !valid_password || !credentials.user.is_active {
            return Err(AppError::Unauthorized);
        }
        self.create_session(credentials.user).await
    }

    pub async fn refresh(&self, refresh_token: String) -> Result<TokenResponse, AppError> {
        validate_refresh_token(&refresh_token).map_err(|_| AppError::Unauthorized)?;
        let replacement = new_refresh_token();
        let session = self
            .sessions
            .rotate(
                refresh_token_hash(&refresh_token),
                refresh_token_hash(&replacement),
            )
            .await?;
        let user = self.active_user(session.user_id).await?;
        let (access_token, expires_in) =
            self.tokens
                .issue_access(user.id, session.id, session.expires_at)?;
        Ok(TokenResponse {
            access_token,
            refresh_token: replacement,
            token_type: "Bearer",
            expires_in,
            user,
        })
    }

    pub async fn logout(&self, principal: &Principal) -> Result<(), AppError> {
        self.sessions
            .revoke(principal.session_id, principal.user_id)
            .await
    }

    pub async fn authenticate_access(&self, token: &str) -> Result<Principal, AppError> {
        let claims = self.tokens.verify_access(token)?;
        if !self.sessions.is_active(claims.sid, claims.sub).await? {
            return Err(AppError::Unauthorized);
        }
        // Read current role and status. Role changes and account deactivation take
        // effect immediately instead of trusting stale authorization in a JWT.
        let user = self.active_user(claims.sub).await?;
        Ok(Principal {
            user_id: user.id,
            role: user.role,
            session_id: claims.sid,
        })
    }

    async fn active_user(&self, id: Uuid) -> Result<User, AppError> {
        let user = self
            .users
            .get_by_id(id)
            .await
            .map_err(|error| match error {
                AppError::NotFound(_) => AppError::Unauthorized,
                other => other,
            })?;
        if !user.is_active {
            return Err(AppError::Unauthorized);
        }
        Ok(user)
    }

    async fn create_session(&self, user: User) -> Result<TokenResponse, AppError> {
        let mut transaction = self.sessions.begin().await?;
        let response = self
            .create_session_in_transaction(user, &mut transaction)
            .await?;
        transaction.commit().await?;
        Ok(response)
    }

    async fn create_session_in_transaction(
        &self,
        user: User,
        transaction: &mut Transaction<'_, Postgres>,
    ) -> Result<TokenResponse, AppError> {
        let session_id = Uuid::new_v4();
        let lifetime = Duration::try_seconds(self.refresh_ttl_seconds).ok_or(AppError::Internal)?;
        let expires_at = Utc::now()
            .checked_add_signed(lifetime)
            .ok_or(AppError::Internal)?;
        let refresh_token = new_refresh_token();
        let (access_token, expires_in) =
            self.tokens.issue_access(user.id, session_id, expires_at)?;
        self.sessions
            .create_in_transaction(
                transaction,
                session_id,
                user.id,
                expires_at,
                refresh_token_hash(&refresh_token),
            )
            .await?;
        Ok(TokenResponse {
            access_token,
            refresh_token,
            token_type: "Bearer",
            expires_in,
            user,
        })
    }
}
