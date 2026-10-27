//! Modular monolith: HTTP adapters delegate to services; repositories own SQL.
pub mod bootstrap;
pub mod config;
pub mod database;
pub mod middleware;
pub mod modules;
pub mod shared;
