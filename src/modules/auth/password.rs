use std::sync::Arc;

use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use rand::rngs::OsRng;
use tokio::sync::Semaphore;

use crate::shared::AppError;

#[derive(Clone)]
pub(super) struct PasswordService {
    permits: Arc<Semaphore>,
}

impl PasswordService {
    pub fn new(concurrency: usize) -> Result<Self, AppError> {
        if concurrency == 0 {
            return Err(AppError::Internal);
        }
        Ok(Self {
            permits: Arc::new(Semaphore::new(concurrency)),
        })
    }

    pub async fn hash(&self, password: String) -> Result<String, AppError> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::TooManyRequests)?;
        tokio::task::spawn_blocking(move || {
            // Hold the permit inside the blocking task, including if its caller is cancelled.
            let _permit = permit;
            let salt = SaltString::generate(&mut OsRng);
            argon2()?
                .hash_password(password.as_bytes(), &salt)
                .map(|hash| hash.to_string())
                .map_err(|_| AppError::Internal)
        })
        .await
        .map_err(|_| AppError::Internal)?
    }

    pub async fn verify(&self, password: String, encoded: String) -> Result<bool, AppError> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| AppError::TooManyRequests)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let hash = PasswordHash::new(&encoded).map_err(|_| AppError::Internal)?;
            match argon2()?.verify_password(password.as_bytes(), &hash) {
                Ok(()) => Ok(true),
                Err(argon2::password_hash::Error::Password) => Ok(false),
                Err(_) => Err(AppError::Internal),
            }
        })
        .await
        .map_err(|_| AppError::Internal)?
    }
}

fn argon2() -> Result<Argon2<'static>, AppError> {
    // Argon2id: 19 MiB, two iterations, one lane (OWASP's minimum configuration).
    let parameters = Params::new(19_456, 2, 1, None).map_err(|_| AppError::Internal)?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn passwords_are_salted_and_verified() {
        let service = PasswordService::new(1).unwrap();
        let password = "a sufficiently long password".to_owned();
        let first = service.hash(password.clone()).await.unwrap();
        let second = service.hash(password.clone()).await.unwrap();
        assert!(first.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert_ne!(first, second);
        assert!(service.verify(password, first.clone()).await.unwrap());
        assert!(
            !service
                .verify("wrong password".to_owned(), first)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn password_work_is_rejected_when_capacity_is_exhausted() {
        let service = PasswordService::new(1).unwrap();
        let _permit = service.permits.clone().acquire_owned().await.unwrap();
        assert!(matches!(
            service
                .hash("a sufficiently long password".to_owned())
                .await,
            Err(AppError::TooManyRequests)
        ));
    }
}
