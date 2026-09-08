use crate::{Result, WireError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseFrame {
    pub event: String,
    pub data: String,
}
/// Incremental SSE framing. Supports LF, CRLF and CR, including split UTF-8 and BOM.
/// Transport chunks must not exceed `MAX_CHUNK`; bounded total bytes prevents endless comments.
pub struct SseDecoder {
    line: Vec<u8>,
    data: String,
    event: String,
    has_data: bool,
    first_line: bool,
    skip_lf: bool,
    total: usize,
    closed: bool,
}
impl Default for SseDecoder {
    fn default() -> Self {
        Self {
            line: Vec::new(),
            data: String::new(),
            event: String::new(),
            has_data: false,
            first_line: true,
            skip_lf: false,
            total: 0,
            closed: false,
        }
    }
}
impl SseDecoder {
    pub const MAX_CHUNK: usize = 1024 * 1024;
    pub const MAX_LINE: usize = 1024 * 1024;
    pub const MAX_EVENT: usize = 2 * 1024 * 1024;
    pub const MAX_STREAM: usize = 32 * 1024 * 1024;
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>> {
        if self.closed {
            return Err(WireError::Closed);
        }
        let result = self.consume(chunk);
        if result.is_err() {
            self.closed = true;
            self.line.clear();
            self.data.clear();
            self.event.clear();
        }
        result
    }
    fn consume(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>> {
        self.total = self
            .total
            .checked_add(chunk.len())
            .ok_or(WireError::LimitExceeded)?;
        if chunk.len() > Self::MAX_CHUNK || self.total > Self::MAX_STREAM {
            return Err(WireError::LimitExceeded);
        }
        let mut frames = Vec::new();
        for byte in chunk {
            if self.skip_lf {
                self.skip_lf = false;
                if *byte == b'\n' {
                    continue;
                }
            }
            if *byte == b'\n' || *byte == b'\r' {
                self.finish_line(&mut frames)?;
                self.skip_lf = *byte == b'\r';
            } else {
                if self.line.len() == Self::MAX_LINE {
                    return Err(WireError::LimitExceeded);
                }
                self.line.push(*byte);
            }
        }
        Ok(frames)
    }
    fn finish_line(&mut self, frames: &mut Vec<SseFrame>) -> Result<()> {
        let bytes = std::mem::take(&mut self.line);
        let mut line = std::str::from_utf8(&bytes).map_err(|_| WireError::InvalidUtf8)?;
        if self.first_line {
            line = line.strip_prefix('\u{feff}').unwrap_or(line);
            self.first_line = false;
        }
        if line.is_empty() {
            if self.has_data {
                self.data.pop();
                frames.push(SseFrame {
                    event: if self.event.is_empty() {
                        "message".into()
                    } else {
                        std::mem::take(&mut self.event)
                    },
                    data: std::mem::take(&mut self.data),
                });
            }
            self.event.clear();
            self.has_data = false;
            return Ok(());
        }
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => {
                if value.len() > 256 {
                    return Err(WireError::LimitExceeded);
                }
                self.event = value.into();
            }
            "data" => {
                if self.data.len() + value.len() + 1 > Self::MAX_EVENT {
                    return Err(WireError::LimitExceeded);
                }
                self.data.push_str(value);
                self.data.push('\n');
                self.has_data = true;
            }
            // No automatic reconnection: ignore id/retry rather than repeat side effects.
            _ => {}
        }
        Ok(())
    }
    /// The protocol normalizer, not an EOF, must confirm its terminal marker.
    pub fn finish(&mut self, protocol_completed: bool) -> Result<()> {
        if self.closed {
            return Err(WireError::Closed);
        }
        self.closed = true;
        let clean = self.line.is_empty() && !self.has_data;
        self.line.clear();
        self.data.clear();
        self.event.clear();
        if clean && protocol_completed {
            Ok(())
        } else {
            Err(WireError::TruncatedStream)
        }
    }
    pub fn cancel(&mut self) {
        self.closed = true;
        self.line.clear();
        self.data.clear();
        self.event.clear();
    }
}
