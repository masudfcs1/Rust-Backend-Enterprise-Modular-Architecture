//! User identity and profile management.
//!
//! Other modules use the service interface instead of querying the users table.

mod dto;
mod handler;
mod model;
mod repository;
mod routes;
mod service;

pub use model::{NewUser, Role, User, UserCredentials};
pub use routes::routes;
pub use service::UsersService;

#[cfg(test)]
mod tests;
