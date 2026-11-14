//! Сервер CRM.

pub mod access;
pub mod admin;
pub mod attachments;
pub mod auth;
pub mod backup;
pub mod chat;
pub mod cli;
pub mod clients;
pub mod config;
pub mod db;
pub mod events;
pub mod http;
pub mod limits;
pub mod logging;
pub mod org;
pub mod recovery;
pub mod secrets;
pub mod server;
pub mod tasks;
pub mod tls;
pub mod web;
#[cfg(windows)]
pub mod winsvc;
