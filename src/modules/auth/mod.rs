//! Authentication, revocable access tokens, and rotating refresh sessions.
mod dto;
mod handlers;
mod password;
mod repository;
mod service;
mod token;

pub use dto::{LoginRequest, RefreshRequest, RegisterRequest, TokenResponse};
pub use handlers::routes;
pub use service::{AuthService, Principal};
