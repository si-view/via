use tokio_util::codec::LengthDelimitedCodec;

/// 4-byte little-endian u32 length prefix, max 64 MiB body.
///
/// Frame layout on the Unix socket (send ↔ serve):
///
///   ┌──────────────────┬───────────────────────────┐
///   │  4B LE u32 len   │  len bytes UTF-8 JSON      │
///   └──────────────────┴───────────────────────────┘
///
/// Chosen over newline-delimited framing to handle embedded newlines in
/// SKILL strings without escaping, and to allow future binary payloads.
pub fn new() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .length_field_length(4)
        .little_endian()
        .max_frame_length(64 * 1024 * 1024)
        .new_codec()
}
