// src-tauri/src/util/error.rs
//! 统一错误类型定义

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("Tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("Reqwest error: {0}")]
    Reqwest(reqwest::Error),

    #[error("Invalid state: {0}")]
    InvalidState(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Config parse error: {0}")]
    ConfigParse(String),

    #[error("Subscription validation error: {0}")]
    Subscription(String),

    #[error("{0}")]
    Other(String),
}

/// reqwest 的 Display 会追加 ` for url (完整URL)`，订阅地址里的 token 会因此进入
/// 日志、诊断包与前端错误提示。统一在转换处剥离 URL（`redact_url_for_log` 的约定）。
impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Error::Reqwest(e.without_url())
    }
}

impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Error::Other(e.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
}

/// Tauri 命令要求错误类型实现 `Serialize`（`E: Into<InvokeError>`）。
/// 序列化为纯字符串即可，前端拿到的是可读的错误信息。
impl serde::Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reqwest_errors_never_carry_the_request_url() {
        // 连接 127.0.0.1:1 必然失败；URL 中的 token 不得出现在错误文本里。
        let err = reqwest::Client::new()
            .get("http://127.0.0.1:1/api/v1/client/SECRET_TOKEN?key=SECRET_KEY")
            .send()
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("SECRET_TOKEN"),
            "premise: raw error leaks url"
        );
        let text = Error::from(err).to_string();
        assert!(
            !text.contains("SECRET_TOKEN") && !text.contains("SECRET_KEY"),
            "{}",
            text
        );
    }
}
