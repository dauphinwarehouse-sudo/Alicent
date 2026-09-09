use std::io::Cursor;

use alicent_document_io::{ArchiveBudget, ArchiveEntry, ArchiveLimits, DocumentIoError};

fn entry<'a>(path: &'a str, compressed: u64, expanded: u64) -> ArchiveEntry<'a> {
    ArchiveEntry {
        path,
        compressed_bytes: compressed,
        declared_expanded_bytes: expanded,
        is_directory: false,
    }
}

#[test]
fn safe_paths_and_directories_are_normalized() {
    let budget = ArchiveBudget::new(ArchiveLimits::default()).unwrap();
    assert_eq!(
        budget
            .validate_path("word/document.xml", false)
            .unwrap()
            .as_str(),
        "word/document.xml"
    );
    assert_eq!(
        budget.validate_path("OEBPS/text/", true).unwrap().as_str(),
        "OEBPS/text"
    );
}

#[test]
fn traversal_absolute_windows_and_ambiguous_paths_are_rejected() {
    let budget = ArchiveBudget::new(ArchiveLimits::default()).unwrap();
    for path in [
        "../secret",
        "word/../secret",
        "/absolute",
        "\\\\server\\share",
        "C:/windows",
        "word\\document.xml",
        "word//document.xml",
        "word/./document.xml",
        "word/document.xml:stream",
        "word/\0document.xml",
    ] {
        assert!(
            matches!(
                budget.validate_path(path, false),
                Err(DocumentIoError::InvalidArchivePath { .. })
            ),
            "{path:?} should be rejected"
        );
    }
}

#[test]
fn metadata_zip_bomb_limits_are_enforced_before_copying() {
    let limits = ArchiveLimits {
        max_entries: 1,
        max_entry_expanded_bytes: 20,
        max_total_expanded_bytes: 20,
        max_compression_ratio: 4,
        ..ArchiveLimits::default()
    };
    let mut budget = ArchiveBudget::new(limits).unwrap();
    let ratio_error = budget
        .copy_entry(entry("bomb", 1, 5), Cursor::new(b"12345"), Vec::new(), 8)
        .unwrap_err();
    assert!(matches!(
        ratio_error,
        DocumentIoError::ArchiveCompressionRatioExceeded { limit: 4 }
    ));

    let mut budget = ArchiveBudget::new(limits).unwrap();
    let size_error = budget
        .copy_entry(
            entry("huge", 20, 21),
            Cursor::new(vec![0; 21]),
            Vec::new(),
            8,
        )
        .unwrap_err();
    assert!(matches!(
        size_error,
        DocumentIoError::ArchiveEntryTooLarge { limit: 20 }
    ));
}

#[test]
fn actual_decompressed_bytes_are_bounded_even_when_metadata_lies() {
    let limits = ArchiveLimits {
        max_entry_expanded_bytes: 8,
        max_total_expanded_bytes: 8,
        max_compression_ratio: 100,
        ..ArchiveLimits::default()
    };
    let mut budget = ArchiveBudget::new(limits).unwrap();
    let error = budget
        .copy_entry(
            entry("forged", 10, 4),
            Cursor::new(b"123456789"),
            Vec::new(),
            3,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        DocumentIoError::ArchiveEntryTooLarge { limit: 8 }
    ));
}

#[test]
fn declared_and_actual_sizes_must_match() {
    let mut budget = ArchiveBudget::new(ArchiveLimits::default()).unwrap();
    let error = budget
        .copy_entry(entry("short", 10, 5), Cursor::new(b"1234"), Vec::new(), 2)
        .unwrap_err();
    assert!(matches!(
        error,
        DocumentIoError::ArchiveSizeMismatch {
            declared: 5,
            actual: 4
        }
    ));
}

#[test]
fn cumulative_entry_and_expanded_limits_are_enforced() {
    let limits = ArchiveLimits {
        max_entries: 2,
        max_entry_expanded_bytes: 8,
        max_total_expanded_bytes: 8,
        max_compression_ratio: 10,
        ..ArchiveLimits::default()
    };
    let mut budget = ArchiveBudget::new(limits).unwrap();
    budget
        .copy_entry(entry("a", 4, 4), Cursor::new(b"1234"), Vec::new(), 2)
        .unwrap();
    budget
        .copy_entry(entry("b", 4, 4), Cursor::new(b"5678"), Vec::new(), 2)
        .unwrap();
    let error = budget
        .copy_entry(entry("c", 1, 0), Cursor::new([]), Vec::new(), 2)
        .unwrap_err();
    assert!(matches!(
        error,
        DocumentIoError::TooManyArchiveEntries { limit: 2 }
    ));
}

#[test]
fn path_depth_and_path_byte_limits_are_enforced() {
    let budget = ArchiveBudget::new(ArchiveLimits {
        max_path_bytes: 8,
        max_path_depth: 2,
        ..ArchiveLimits::default()
    })
    .unwrap();
    assert!(matches!(
        budget.validate_path("a/b/c", false),
        Err(DocumentIoError::InvalidArchivePath { .. })
    ));
    assert!(matches!(
        budget.validate_path("123456789", false),
        Err(DocumentIoError::InvalidArchivePath { .. })
    ));
}
