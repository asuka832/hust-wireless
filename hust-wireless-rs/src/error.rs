#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),

    #[error("{0}")]
    LoginUrlRequired(String),

    #[error("I/O 错误：{0}")]
    Io(#[from] std::io::Error),

    #[error("网络错误：{0}")]
    Network(#[from] reqwest::Error),

    #[error("数据格式错误：{0}")]
    Json(#[from] serde_json::Error),

    #[error("网址格式无效：{0}")]
    Url(#[from] url::ParseError),

    #[error("操作已取消")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, AppError>;

impl AppError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}
