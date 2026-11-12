// src-tauri/src/core/mod.rs
//! Core module - mihomo process management

pub mod app_controller;
pub mod config;
pub mod controller;
pub mod health;
pub mod lifecycle;
pub mod logs;
pub mod manager;
pub mod patrol;
mod pipe_http;
pub mod runtime;
pub mod spawn;
pub mod supervisor;
