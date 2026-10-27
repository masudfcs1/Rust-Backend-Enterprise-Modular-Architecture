use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{config::AuthConfig, shared::AppError};

#[derive(Clone)]
pub(super) struct TokenCodec {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    issuer: String,
    audience: String,
    access_ttl_seconds: u64,
    validation: Validation,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct AccessClaims {
    pub sub: Uuid,
    pub sid: Uuid,
    pub jti: Uuid,
    pub iss: String,
    pub aud: String,
    pub iat: u64,
    pub nbf: u64,
    pub exp: u64,
}

impl TokenCodec {
    pub fn new(config: &AuthConfig) -> Result<Self, AppError> {
        if config.jwt_secret.len() < 32
            || config.jwt_issuer.trim().is_empty()
            || config.jwt_audience.trim().is_empty()
            || config.access_ttl_seconds == 0
        {
            return Err(AppError::Internal);
        }
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[&config.jwt_issuer]);
        validation.set_audience(&[&config.jwt_audience]);
        validation.set_required_spec_claims(&["sub", "exp", "iat", "nbf", "iss", "aud"]);
        validation.validate_nbf = true;
        validation.leeway = 0;

        Ok(Self {
            encoding_key: EncodingKey::from_secret(config.jwt_secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(config.jwt_secret.as_bytes()),
            issuer: config.jwt_issuer.clone(),
            audience: config.jwt_audience.clone(),
            access_ttl_seconds: config.access_ttl_seconds,
            validation,
        })
    }

    pub fn issue_access(
        &self,
        user_id: Uuid,
        session_id: Uuid,
        session_expires_at: DateTime<Utc>,
    ) -> Result<(String, u64), AppError> {
        let now = u64::try_from(Utc::now().timestamp()).map_err(|_| AppError::Internal)?;
        let session_expiry =
            u64::try_from(session_expires_at.timestamp()).map_err(|_| AppError::Unauthorized)?;
        let expires_at = now
            .checked_add(self.access_ttl_seconds)
            .ok_or(AppError::Internal)?
            .min(session_expiry);
        if expires_at <= now {
            return Err(AppError::Unauthorized);
        }
        let claims = AccessClaims {
            sub: user_id,
            sid: session_id,
            jti: Uuid::new_v4(),
            iss: self.issuer.clone(),
            aud: self.audience.clone(),
            iat: now,
            nbf: now,
            exp: expires_at,
        };
        let token = encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(|_| AppError::Internal)?;
        Ok((token, expires_at - now))
    }

    pub fn verify_access(&self, token: &str) -> Result<AccessClaims, AppError> {
        let claims = decode::<AccessClaims>(token, &self.decoding_key, &self.validation)
            .map_err(|_| AppError::Unauthorized)?
            .claims;
        let now = u64::try_from(Utc::now().timestamp()).map_err(|_| AppError::Internal)?;
        // Explicitly enforce the expiry boundary and issuance order as well as JWT validation.
        if claims.exp <= now || claims.iat > now || claims.exp <= claims.iat {
            return Err(AppError::Unauthorized);
        }
        Ok(claims)
    }
}

pub(super) fn new_refresh_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(64);
    for byte in bytes {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    token
}

pub(super) fn refresh_token_hash(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    fn config() -> AuthConfig {
        AuthConfig {
            jwt_secret: "a-test-secret-with-at-least-32-bytes".to_owned(),
            jwt_issuer: "enterprise-test".to_owned(),
            jwt_audience: "enterprise-test-client".to_owned(),
            access_ttl_seconds: 900,
            refresh_ttl_seconds: 86_400,
            password_concurrency: 1,
        }
    }

    fn valid_claims() -> AccessClaims {
        let now = Utc::now().timestamp() as u64;
        AccessClaims {
            sub: Uuid::new_v4(),
            sid: Uuid::new_v4(),
            jti: Uuid::new_v4(),
            iss: config().jwt_issuer,
            aud: config().jwt_audience,
            iat: now,
            nbf: now,
            exp: now + 900,
        }
    }

    fn sign(claims: &AccessClaims, config: &AuthConfig) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            claims,
            &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn valid_tokens_preserve_identity_and_session() {
        let codec = TokenCodec::new(&config()).unwrap();
        let user = Uuid::new_v4();
        let session = Uuid::new_v4();
        let (token, expiry) = codec
            .issue_access(user, session, Utc::now() + Duration::hours(1))
            .unwrap();
        let claims = codec.verify_access(&token).unwrap();
        assert_eq!(claims.sub, user);
        assert_eq!(claims.sid, session);
        assert_eq!(expiry, 900);
    }

    #[test]
    fn invalid_issuer_audience_expiry_and_future_tokens_are_rejected() {
        let config = config();
        let codec = TokenCodec::new(&config).unwrap();
        let mut claims = valid_claims();
        claims.iss = "another-issuer".to_owned();
        assert!(codec.verify_access(&sign(&claims, &config)).is_err());
        let mut claims = valid_claims();
        claims.aud = "another-audience".to_owned();
        assert!(codec.verify_access(&sign(&claims, &config)).is_err());
        let mut claims = valid_claims();
        claims.exp = claims.iat - 1;
        assert!(codec.verify_access(&sign(&claims, &config)).is_err());
        let mut claims = valid_claims();
        claims.nbf = claims.iat + 60;
        assert!(codec.verify_access(&sign(&claims, &config)).is_err());
    }

    #[test]
    fn wrong_secret_and_algorithm_are_rejected() {
        let original_config = config();
        let codec = TokenCodec::new(&original_config).unwrap();
        let mut wrong_config = config();
        wrong_config.jwt_secret = "another-secret-with-at-least-32-bytes".to_owned();
        assert!(
            codec
                .verify_access(&sign(&valid_claims(), &wrong_config))
                .is_err()
        );
        let wrong_algorithm = encode(
            &Header::new(Algorithm::HS384),
            &valid_claims(),
            &EncodingKey::from_secret(original_config.jwt_secret.as_bytes()),
        )
        .unwrap();
        assert!(codec.verify_access(&wrong_algorithm).is_err());
    }

    #[test]
    fn refresh_tokens_are_random_and_stored_as_digests() {
        let first = new_refresh_token();
        let second = new_refresh_token();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
        assert_eq!(refresh_token_hash(&first).len(), 32);
        assert_ne!(refresh_token_hash(&first), refresh_token_hash(&second));
    }
}
