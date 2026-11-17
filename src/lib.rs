//! supermarket-rs — a wire-compatible Rust port of the Rails Supermarket app.

pub mod api;
pub mod auth;
pub mod cache;
pub mod chef_version;
pub mod config;
pub mod db;
pub mod error;
pub mod jobs;
pub mod locales;
pub mod mail;
pub mod middleware;
pub mod query;
pub mod routes;
pub mod session;
pub mod state;
pub mod storage;
pub mod upload;
pub mod views;
pub mod web;
pub mod worker;

pub use state::AppState;
