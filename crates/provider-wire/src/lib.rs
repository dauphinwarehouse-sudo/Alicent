//! Provider wire formats and privacy-preserving HTTP connectivity.
#[rustfmt::skip]
mod anthropic;
#[rustfmt::skip]
mod anthropic_message;
mod chat;
mod integration;
mod json;
mod provider;
mod request;
mod sse;
mod tools;
mod transport;
mod vault;
pub use anthropic::AnthropicDecoder;
pub use anthropic_message::{
    build_anthropic_request, decode_anthropic_message, AnthropicResponse, MAX_RESPONSE_BYTES,
};
pub use chat::*;
pub use integration::*;
pub use provider::*;
pub use request::*;
pub use sse::*;
pub use tools::*;
pub use transport::*;
pub use vault::*;

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

/// Only allowlisted public error codes: never return response bodies, headers
/// or secrets to the caller.
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
/// Policy only; the future transport owns backoff, Retry-After, timeouts and
/// cancellation.
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
