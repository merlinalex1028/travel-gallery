use serde::Serialize;
use serde_json::{json, Value};

pub type Result<T> = std::result::Result<T, AppError>;
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub details: Value,
}
impl AppError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
            details: json!({}),
        }
    }
    pub fn details(mut self, value: Value) -> Self {
        self.details = value;
        self
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AppError {}
impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("INTERNAL", "资料库操作失败").details(json!({"reason":e.to_string()}))
    }
}
impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::new("INTERNAL", "文件操作失败").details(json!({"reason":e.to_string()}))
    }
}
impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::new("INVALID_FILTER", "请求格式不正确").details(json!({"reason":e.to_string()}))
    }
}
pub fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::new("INVALID_FILTER", format!("缺少字段 {key}")))
}
