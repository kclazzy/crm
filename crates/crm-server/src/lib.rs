//! Сервер CRM.

pub mod auth;
pub mod cli;
pub mod config;
pub mod db;
pub mod http;
pub mod logging;
pub mod secrets;
pub mod server;
pub mod tls;
#[cfg(windows)]
pub mod winsvc;
