//! Offline provider wire building blocks. No HTTP, credentials, tool execution or UI claims.
mod chat;
mod json;
mod request;
mod sse;
mod tools;
pub use chat::*;
pub use request::*;
pub use sse::*;
pub use tools::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WireError {
    #[error("Некорректный запрос модели")]
    InvalidRequest,
    #[error("Нарушена последовательность вызовов инструментов")]
    InvalidTranscript,
    #[error("Превышен лимит данных провайдера")]
    LimitExceeded,
    #[error("Некорректный UTF-8 потока")]
    InvalidUtf8,
    #[error("Незавершённый поток провайдера")]
    TruncatedStream,
    #[error("Некорректные аргументы инструмента")]
    InvalidArguments,
    #[error("Некорректный ответ провайдера")]
    InvalidResponse,
    #[error("Неподдерживаемый формат ответа провайдера")]
    UnsupportedResponse,
    #[error("Ответ провайдера не завершён успешно")]
    IncompleteResponse,
    #[error("Ошибка провайдера")]
    ProviderFailure,
    #[error("Поток уже закрыт")]
    Closed,
}
pub type Result<T> = std::result::Result<T, WireError>;

/// Only allowlisted public error codes: never return response bodies/headers/secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpFailure {
    Authentication,
    RateLimited,
    InvalidRequest,
    NotFound,
    Server,
    Other,
}
pub fn classify_http(status: u16) -> HttpFailure {
    match status {
        401 | 403 => HttpFailure::Authentication,
        429 => HttpFailure::RateLimited,
        400 | 422 => HttpFailure::InvalidRequest,
        404 => HttpFailure::NotFound,
        500..=599 => HttpFailure::Server,
        _ => HttpFailure::Other,
    }
}
/// Policy only; the future transport owns backoff, Retry-After, timeouts and cancellation.
pub fn may_retry(
    status: u16,
    attempts: u8,
    response_started: bool,
    side_effect_started: bool,
) -> bool {
    attempts < 2
        && !response_started
        && !side_effect_started
        && matches!(status, 429 | 502 | 503 | 504)
}
