//! Сервер CRM.

pub mod admin;
pub mod auth;
pub mod backup;
pub mod cli;
pub mod config;
pub mod db;
pub mod http;
pub mod logging;
pub mod recovery;
pub mod secrets;
pub mod server;
pub mod tls;
pub mod web;
#[cfg(windows)]
pub mod winsvc;
