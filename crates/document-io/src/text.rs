use std::{
    ffi::OsStr,
    io::{Read, Write},
};

use crate::{DocumentIoError, Result};

const MAX_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFormat {
    Markdown,
    PlainText,
}

impl TextFormat {
    pub fn from_extension(extension: &OsStr) -> Option<Self> {
        let extension = extension.to_str()?;
        if extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown") {
            Some(Self::Markdown)
        } else if extension.eq_ignore_ascii_case("txt") {
            Some(Self::PlainText)
        } else {
            None
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::PlainText => "txt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoLimits {
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub buffer_bytes: usize,
}

impl Default for IoLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_output_bytes: 64 * 1024 * 1024,
            buffer_bytes: 32 * 1024,
        }
    }
}

impl IoLimits {
    fn validate(self) -> Result<()> {
        if self.max_input_bytes == 0 {
            return Err(DocumentIoError::InvalidLimits("max_input_bytes"));
        }
        if self.max_output_bytes == 0 {
            return Err(DocumentIoError::InvalidLimits("max_output_bytes"));
        }
        if self.buffer_bytes == 0 || self.buffer_bytes > MAX_BUFFER_BYTES {
            return Err(DocumentIoError::InvalidLimits("buffer_bytes"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferReport {
    pub format: TextFormat,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

/// Imports UTF-8 Markdown or plain text without normalizing bytes or newlines.
///
/// Valid bytes can already have reached `writer` when a later read fails or
/// contains invalid UTF-8. Use a temporary destination when publication must
/// be atomic.
pub fn import_text<R: Read, W: Write>(
    format: TextFormat,
    reader: R,
    writer: W,
    limits: IoLimits,
) -> Result<TransferReport> {
    transfer_utf8(format, reader, writer, limits)
}

/// Exports UTF-8 Markdown or plain text without normalizing bytes or newlines.
///
/// The caller chooses the semantic conversion before this byte-preserving
/// boundary. This function validates UTF-8 and enforces both transfer limits.
pub fn export_text<R: Read, W: Write>(
    format: TextFormat,
    reader: R,
    writer: W,
    limits: IoLimits,
) -> Result<TransferReport> {
    transfer_utf8(format, reader, writer, limits)
}

fn transfer_utf8<R: Read, W: Write>(
    format: TextFormat,
    mut reader: R,
    mut writer: W,
    limits: IoLimits,
) -> Result<TransferReport> {
    limits.validate()?;

    let mut buffer = vec![0_u8; limits.buffer_bytes];
    let mut pending = Vec::with_capacity(limits.buffer_bytes.saturating_add(3));
    let mut input_bytes = 0_u64;
    let mut output_bytes = 0_u64;

    loop {
        let read = reader.read(&mut buffer).map_err(DocumentIoError::Read)?;
        if read == 0 {
            break;
        }

        input_bytes =
            input_bytes
                .checked_add(read as u64)
                .ok_or(DocumentIoError::InputTooLarge {
                    limit: limits.max_input_bytes,
                })?;
        if input_bytes > limits.max_input_bytes {
            return Err(DocumentIoError::InputTooLarge {
                limit: limits.max_input_bytes,
            });
        }

        pending.extend_from_slice(&buffer[..read]);
        match std::str::from_utf8(&pending) {
            Ok(_) => {
                write_checked(&mut writer, &pending, &mut output_bytes, limits)?;
                pending.clear();
            }
            Err(error) if error.error_len().is_none() => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    write_checked(&mut writer, &pending[..valid], &mut output_bytes, limits)?;
                    pending.drain(..valid);
                }
                debug_assert!(pending.len() <= 3);
            }
            Err(error) => {
                let pending_offset = input_bytes - pending.len() as u64;
                return Err(DocumentIoError::InvalidUtf8 {
                    offset: pending_offset + error.valid_up_to() as u64,
                });
            }
        }
    }

    if !pending.is_empty() {
        return Err(DocumentIoError::InvalidUtf8 {
            offset: input_bytes - pending.len() as u64,
        });
    }

    Ok(TransferReport {
        format,
        input_bytes,
        output_bytes,
    })
}

fn write_checked<W: Write>(
    writer: &mut W,
    bytes: &[u8],
    output_bytes: &mut u64,
    limits: IoLimits,
) -> Result<()> {
    let next =
        output_bytes
            .checked_add(bytes.len() as u64)
            .ok_or(DocumentIoError::OutputTooLarge {
                limit: limits.max_output_bytes,
            })?;
    if next > limits.max_output_bytes {
        return Err(DocumentIoError::OutputTooLarge {
            limit: limits.max_output_bytes,
        });
    }
    writer.write_all(bytes).map_err(DocumentIoError::Write)?;
    *output_bytes = next;
    Ok(())
}
