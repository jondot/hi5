#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("github api error: {0}")]
    GitHub(String),
    #[error("authentication failed: {0}")]
    Auth(String),
    #[error("token is invalid or expired")]
    Unauthorized,
    #[error("rate limited until {0}")]
    RateLimited(i64),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
}

pub type Result<T> = std::result::Result<T, AppError>;
