// src-tauri/src/core/mod.rs
//! Core module - mihomo process management

pub mod app_controller;
pub mod config;
pub mod controller;
pub mod health;
pub mod integrity;
pub mod lifecycle;
pub mod logs;
pub mod manager;
pub mod node_health;
pub mod patrol;
mod pipe_http;
pub mod profile_failover;
pub mod runtime;
pub mod spawn;
pub mod supervisor;
