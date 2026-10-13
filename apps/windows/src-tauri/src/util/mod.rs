// src-tauri/src/util/mod.rs
//! 工具模块

pub mod atomic;
pub mod autostart;
pub mod elevation;
pub mod error;
pub mod fetch;
pub mod logging;
pub mod normalizer;
pub mod paths;
pub mod process;
pub mod temp;
pub mod uri_list;

pub use error::Error;
