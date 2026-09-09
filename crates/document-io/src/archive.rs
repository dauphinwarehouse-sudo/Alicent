use std::{
    borrow::Cow,
    io::{Read, Write},
};

use crate::{DocumentIoError, Result};

const MAX_BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveLimits {
    pub max_entries: u64,
    pub max_entry_expanded_bytes: u64,
    pub max_total_expanded_bytes: u64,
    pub max_compression_ratio: u64,
    pub max_path_bytes: usize,
    pub max_path_depth: usize,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: 4_096,
            max_entry_expanded_bytes: 64 * 1024 * 1024,
            max_total_expanded_bytes: 256 * 1024 * 1024,
            max_compression_ratio: 100,
            max_path_bytes: 1_024,
            max_path_depth: 32,
        }
    }
}

impl ArchiveLimits {
    fn validate(self) -> Result<()> {
        if self.max_entries == 0 {
            return Err(DocumentIoError::InvalidLimits("max_entries"));
        }
        if self.max_entry_expanded_bytes == 0 {
            return Err(DocumentIoError::InvalidLimits("max_entry_expanded_bytes"));
        }
        if self.max_total_expanded_bytes == 0 {
            return Err(DocumentIoError::InvalidLimits("max_total_expanded_bytes"));
        }
        if self.max_compression_ratio == 0 {
            return Err(DocumentIoError::InvalidLimits("max_compression_ratio"));
        }
        if self.max_path_bytes == 0 {
            return Err(DocumentIoError::InvalidLimits("max_path_bytes"));
        }
        if self.max_path_depth == 0 {
            return Err(DocumentIoError::InvalidLimits("max_path_depth"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArchiveEntry<'a> {
    /// ZIP member name. Forward slash is the only accepted separator.
    pub path: &'a str,
    pub compressed_bytes: u64,
    pub declared_expanded_bytes: u64,
    pub is_directory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedArchivePath<'a>(Cow<'a, str>);

impl<'a> ValidatedArchivePath<'a> {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntryReport<'a> {
    pub path: ValidatedArchivePath<'a>,
    pub expanded_bytes: u64,
}

/// Shared archive budget for future DOCX/EPUB ZIP adapters.
///
/// Metadata is checked before decompression and actual bytes are counted while
/// copying, so forged ZIP size fields cannot bypass the expanded-size limits.
#[derive(Debug)]
pub struct ArchiveBudget {
    limits: ArchiveLimits,
    entries: u64,
    declared_expanded_bytes: u64,
    actual_expanded_bytes: u64,
}

impl ArchiveBudget {
    pub fn new(limits: ArchiveLimits) -> Result<Self> {
        limits.validate()?;
        Ok(Self {
            limits,
            entries: 0,
            declared_expanded_bytes: 0,
            actual_expanded_bytes: 0,
        })
    }

    pub fn entries(&self) -> u64 {
        self.entries
    }

    pub fn actual_expanded_bytes(&self) -> u64 {
        self.actual_expanded_bytes
    }

    pub fn validate_path<'a>(
        &self,
        path: &'a str,
        is_directory: bool,
    ) -> Result<ValidatedArchivePath<'a>> {
        validate_archive_path(path, is_directory, self.limits)
    }

    pub fn copy_entry<'a, R: Read, W: Write>(
        &mut self,
        entry: ArchiveEntry<'a>,
        mut reader: R,
        mut writer: W,
        buffer_bytes: usize,
    ) -> Result<ArchiveEntryReport<'a>> {
        if buffer_bytes == 0 || buffer_bytes > MAX_BUFFER_BYTES {
            return Err(DocumentIoError::InvalidLimits("buffer_bytes"));
        }

        let path = self.register(entry)?;
        if entry.is_directory {
            return Ok(ArchiveEntryReport {
                path,
                expanded_bytes: 0,
            });
        }

        let ratio_limit = ratio_limit(entry.compressed_bytes, self.limits.max_compression_ratio);
        let mut entry_actual = 0_u64;
        let mut buffer = vec![0_u8; buffer_bytes];

        loop {
            let read = reader.read(&mut buffer).map_err(DocumentIoError::Read)?;
            if read == 0 {
                break;
            }
            entry_actual = entry_actual.checked_add(read as u64).ok_or(
                DocumentIoError::ArchiveEntryTooLarge {
                    limit: self.limits.max_entry_expanded_bytes,
                },
            )?;
            if entry_actual > self.limits.max_entry_expanded_bytes {
                return Err(DocumentIoError::ArchiveEntryTooLarge {
                    limit: self.limits.max_entry_expanded_bytes,
                });
            }
            if entry_actual > ratio_limit {
                return Err(DocumentIoError::ArchiveCompressionRatioExceeded {
                    limit: self.limits.max_compression_ratio,
                });
            }

            let total = self.actual_expanded_bytes.checked_add(read as u64).ok_or(
                DocumentIoError::ArchiveExpandedTooLarge {
                    limit: self.limits.max_total_expanded_bytes,
                },
            )?;
            if total > self.limits.max_total_expanded_bytes {
                return Err(DocumentIoError::ArchiveExpandedTooLarge {
                    limit: self.limits.max_total_expanded_bytes,
                });
            }

            writer
                .write_all(&buffer[..read])
                .map_err(DocumentIoError::Write)?;
            self.actual_expanded_bytes = total;
        }

        if entry_actual != entry.declared_expanded_bytes {
            return Err(DocumentIoError::ArchiveSizeMismatch {
                declared: entry.declared_expanded_bytes,
                actual: entry_actual,
            });
        }

        Ok(ArchiveEntryReport {
            path,
            expanded_bytes: entry_actual,
        })
    }

    fn register<'a>(&mut self, entry: ArchiveEntry<'a>) -> Result<ValidatedArchivePath<'a>> {
        let path = self.validate_path(entry.path, entry.is_directory)?;
        let entries =
            self.entries
                .checked_add(1)
                .ok_or(DocumentIoError::TooManyArchiveEntries {
                    limit: self.limits.max_entries,
                })?;
        if entries > self.limits.max_entries {
            return Err(DocumentIoError::TooManyArchiveEntries {
                limit: self.limits.max_entries,
            });
        }
        if entry.declared_expanded_bytes > self.limits.max_entry_expanded_bytes {
            return Err(DocumentIoError::ArchiveEntryTooLarge {
                limit: self.limits.max_entry_expanded_bytes,
            });
        }
        if entry.declared_expanded_bytes
            > ratio_limit(entry.compressed_bytes, self.limits.max_compression_ratio)
        {
            return Err(DocumentIoError::ArchiveCompressionRatioExceeded {
                limit: self.limits.max_compression_ratio,
            });
        }
        let declared_total = self
            .declared_expanded_bytes
            .checked_add(entry.declared_expanded_bytes)
            .ok_or(DocumentIoError::ArchiveExpandedTooLarge {
                limit: self.limits.max_total_expanded_bytes,
            })?;
        if declared_total > self.limits.max_total_expanded_bytes {
            return Err(DocumentIoError::ArchiveExpandedTooLarge {
                limit: self.limits.max_total_expanded_bytes,
            });
        }
        self.entries = entries;
        self.declared_expanded_bytes = declared_total;
        Ok(path)
    }
}

fn ratio_limit(compressed_bytes: u64, max_compression_ratio: u64) -> u64 {
    compressed_bytes.saturating_mul(max_compression_ratio)
}

fn validate_archive_path(
    path: &str,
    is_directory: bool,
    limits: ArchiveLimits,
) -> Result<ValidatedArchivePath<'_>> {
    if path.is_empty() {
        return Err(invalid_path("empty path"));
    }
    if path.len() > limits.max_path_bytes {
        return Err(invalid_path("path is too long"));
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(invalid_path("absolute path"));
    }
    if path.contains('\\') {
        return Err(invalid_path("backslash separator"));
    }
    if path.chars().any(char::is_control) {
        return Err(invalid_path("control character"));
    }

    let normalized = if is_directory {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        if path.ends_with('/') {
            return Err(invalid_path("file path has a trailing separator"));
        }
        path
    };
    if normalized.is_empty() {
        return Err(invalid_path("empty path"));
    }

    let mut depth = 0_usize;
    for component in normalized.split('/') {
        if component.is_empty() {
            return Err(invalid_path("empty component"));
        }
        if component == "." || component == ".." {
            return Err(invalid_path("relative traversal component"));
        }
        if component.contains(':') {
            return Err(invalid_path("drive prefix or alternate data stream"));
        }
        depth = depth
            .checked_add(1)
            .ok_or_else(|| invalid_path("path is too deep"))?;
        if depth > limits.max_path_depth {
            return Err(invalid_path("path is too deep"));
        }
    }

    let path = if normalized.len() == path.len() {
        Cow::Borrowed(normalized)
    } else {
        Cow::Owned(normalized.to_owned())
    };
    Ok(ValidatedArchivePath(path))
}

fn invalid_path(reason: &'static str) -> DocumentIoError {
    DocumentIoError::InvalidArchivePath { reason }
}
