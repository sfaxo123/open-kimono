use geni_uart::{rxline::LineReader, ByteSource, ByteSourceError, RxError};
struct Fragment {
    bytes: std::collections::VecDeque<Option<u8>>,
}
impl ByteSource for Fragment {
    fn read_byte(&mut self) -> Result<u8, ByteSourceError> {
        self.bytes
            .pop_front()
            .flatten()
            .ok_or(ByteSourceError::Rx(RxError::WouldReadNothing))
    }
}
#[test]
fn pending_fragment_is_not_a_completed_line() {
    let mut source = Fragment {
        bytes: [Some(b'a'), Some(b'b'), None, Some(b'c'), Some(b'\n')].into(),
    };
    let mut reader = LineReader::new(&mut source);
    let mut buf = [0u8; 16];
    assert_eq!(
        reader.read_line(&mut buf).unwrap(),
        0,
        "FIFO gap must not complete partial line"
    );
    assert_eq!(reader.read_line(&mut buf).unwrap(), 3);
    assert_eq!(&buf[..3], b"abc");
}
#[test]
fn smaller_completion_buffer_must_not_silently_drop_pending_bytes() {
    let mut source = Fragment {
        bytes: [
            Some(b'a'),
            Some(b'b'),
            Some(b'c'),
            Some(b'd'),
            None,
            Some(b'\n'),
        ]
        .into(),
    };
    let mut reader = LineReader::new(&mut source);
    let mut large = [0u8; 8];
    assert_eq!(reader.read_line(&mut large).unwrap(), 0);
    let mut small = [0u8; 2];
    assert!(
        reader.read_line(&mut small).is_err(),
        "four pending bytes completed into two-byte buffer without an explicit capacity error"
    );
}
