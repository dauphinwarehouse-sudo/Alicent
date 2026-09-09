//! Bounded, streaming document import/export primitives.
//!
//! This crate intentionally has no UI, project repository, editor, provider,
//! archive/move, story graph, or tool-runtime integration.

#![forbid(unsafe_code)]

mod archive;
mod text;

pub use archive::{
    ArchiveBudget, ArchiveEntry, ArchiveEntryReport, ArchiveLimits, ValidatedArchivePath,
};
pub use text::{export_text, import_text, IoLimits, TextFormat, TransferReport};

use std::{fmt, io};

pub type Result<T> = std::result::Result<T, DocumentIoError>;

#[derive(Debug)]
pub enum DocumentIoError {
    InvalidLimits(&'static str),
    InputTooLarge { limit: u64 },
    OutputTooLarge { limit: u64 },
    InvalidUtf8 { offset: u64 },
    InvalidArchivePath { reason: &'static str },
    TooManyArchiveEntries { limit: u64 },
    ArchiveEntryTooLarge { limit: u64 },
    ArchiveExpandedTooLarge { limit: u64 },
    ArchiveCompressionRatioExceeded { limit: u64 },
    ArchiveSizeMismatch { declared: u64, actual: u64 },
    Read(io::Error),
    Write(io::Error),
}

impl fmt::Display for DocumentIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimits(name) => write!(formatter, "invalid limit: {name}"),
            Self::InputTooLarge { limit } => {
                write!(formatter, "input exceeds the {limit}-byte limit")
            }
            Self::OutputTooLarge { limit } => {
                write!(formatter, "output exceeds the {limit}-byte limit")
            }
            Self::InvalidUtf8 { offset } => {
                write!(formatter, "invalid UTF-8 at byte offset {offset}")
            }
            Self::InvalidArchivePath { reason } => {
                write!(formatter, "unsafe archive path: {reason}")
            }
            Self::TooManyArchiveEntries { limit } => {
                write!(formatter, "archive exceeds the {limit}-entry limit")
            }
            Self::ArchiveEntryTooLarge { limit } => {
                write!(formatter, "archive entry exceeds the {limit}-byte limit")
            }
            Self::ArchiveExpandedTooLarge { limit } => {
                write!(
                    formatter,
                    "archive expanded data exceeds the {limit}-byte limit"
                )
            }
            Self::ArchiveCompressionRatioExceeded { limit } => {
                write!(
                    formatter,
                    "archive entry exceeds the {limit}:1 compression-ratio limit"
                )
            }
            Self::ArchiveSizeMismatch { declared, actual } => {
                write!(
                    formatter,
                    "archive entry declared {declared} bytes but produced {actual}"
                )
            }
            Self::Read(error) => write!(formatter, "document read failed: {error}"),
            Self::Write(error) => write!(formatter, "document write failed: {error}"),
        }
    }
}

impl std::error::Error for DocumentIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Read(error) | Self::Write(error) => Some(error),
            _ => None,
        }
    }
}
