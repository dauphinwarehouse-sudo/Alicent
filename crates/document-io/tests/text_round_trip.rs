use std::{
    ffi::OsStr,
    io::{self, Cursor, Read, Write},
};

use alicent_document_io::{export_text, import_text, DocumentIoError, IoLimits, TextFormat};

const MARKDOWN: &[u8] = include_bytes!("fixtures/unicode.md");
const TEXT: &[u8] = include_bytes!("fixtures/unicode.txt");

#[test]
fn unicode_markdown_round_trips_across_every_small_chunk_boundary() {
    for chunk in 1..=17 {
        let mut imported = Vec::new();
        let report = import_text(
            TextFormat::Markdown,
            Cursor::new(MARKDOWN),
            &mut imported,
            IoLimits {
                buffer_bytes: chunk,
                ..IoLimits::default()
            },
        )
        .unwrap();
        assert_eq!(report.input_bytes, MARKDOWN.len() as u64);
        assert_eq!(imported, MARKDOWN);

        let mut exported = Vec::new();
        export_text(
            TextFormat::Markdown,
            Cursor::new(&imported),
            &mut exported,
            IoLimits {
                buffer_bytes: chunk,
                ..IoLimits::default()
            },
        )
        .unwrap();
        assert_eq!(exported, MARKDOWN);
    }
}

#[test]
fn unicode_plain_text_round_trips_without_newline_normalization() {
    let mut output = Vec::new();
    import_text(
        TextFormat::PlainText,
        Cursor::new(TEXT),
        &mut output,
        IoLimits {
            buffer_bytes: 2,
            ..IoLimits::default()
        },
    )
    .unwrap();
    assert_eq!(output, TEXT);
}

#[test]
fn format_extensions_are_explicit_and_ascii_case_insensitive() {
    assert_eq!(
        TextFormat::from_extension(OsStr::new("MARKDOWN")),
        Some(TextFormat::Markdown)
    );
    assert_eq!(
        TextFormat::from_extension(OsStr::new("Txt")),
        Some(TextFormat::PlainText)
    );
    assert_eq!(TextFormat::from_extension(OsStr::new("pdf")), None);
}

#[test]
fn invalid_utf8_reports_the_absolute_byte_offset() {
    let input = b"valid \xF0\x9F\x98\x80 then \xF0\x28\x8C\x28";
    let error = import_text(
        TextFormat::PlainText,
        Cursor::new(input),
        Vec::new(),
        IoLimits {
            buffer_bytes: 3,
            ..IoLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, DocumentIoError::InvalidUtf8 { offset: 16 }));
}

#[test]
fn truncated_utf8_is_rejected() {
    let error = import_text(
        TextFormat::PlainText,
        Cursor::new(b"text \xE2\x82"),
        Vec::new(),
        IoLimits {
            buffer_bytes: 1,
            ..IoLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(error, DocumentIoError::InvalidUtf8 { offset: 5 }));
}

#[test]
fn input_and_output_limits_are_independent() {
    let input_error = import_text(
        TextFormat::PlainText,
        Cursor::new(b"12345"),
        Vec::new(),
        IoLimits {
            max_input_bytes: 4,
            ..IoLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        input_error,
        DocumentIoError::InputTooLarge { limit: 4 }
    ));

    let output_error = export_text(
        TextFormat::PlainText,
        Cursor::new(b"12345"),
        Vec::new(),
        IoLimits {
            max_output_bytes: 4,
            ..IoLimits::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        output_error,
        DocumentIoError::OutputTooLarge { limit: 4 }
    ));
}

#[test]
fn read_requests_never_exceed_the_configured_buffer() {
    let reader = RequestTrackingReader {
        remaining: 100_000,
        max_requested: 0,
    };
    let mut writer = io::sink();
    import_text(
        TextFormat::PlainText,
        reader,
        &mut writer,
        IoLimits {
            buffer_bytes: 31,
            ..IoLimits::default()
        },
    )
    .unwrap();
}

#[test]
fn invalid_or_unbounded_buffer_sizes_are_rejected() {
    for buffer_bytes in [0, 1024 * 1024 + 1] {
        let error = import_text(
            TextFormat::PlainText,
            Cursor::new(b"text"),
            Vec::new(),
            IoLimits {
                buffer_bytes,
                ..IoLimits::default()
            },
        )
        .unwrap_err();
        assert!(matches!(
            error,
            DocumentIoError::InvalidLimits("buffer_bytes")
        ));
    }
}

struct RequestTrackingReader {
    remaining: usize,
    max_requested: usize,
}

impl Read for RequestTrackingReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.max_requested = self.max_requested.max(buffer.len());
        assert!(buffer.len() <= 31);
        if self.remaining == 0 {
            return Ok(0);
        }
        let read = self.remaining.min(buffer.len());
        buffer[..read].fill(b'x');
        self.remaining -= read;
        Ok(read)
    }
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("synthetic write failure"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn writer_errors_keep_their_source() {
    let error = export_text(
        TextFormat::PlainText,
        Cursor::new(b"text"),
        FailingWriter,
        IoLimits::default(),
    )
    .unwrap_err();
    assert!(matches!(error, DocumentIoError::Write(_)));
}
