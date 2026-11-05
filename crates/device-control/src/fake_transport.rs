//! Deterministic scripted fake transport for the control-envelope v1 slice.
//!
//! Serves a fixed script of already-encoded envelope frames (2 log frames
//! and 1 screenshot frame). Frames are BORROWED from the script on the serve
//! path — no per-frame heap allocation. An unsupported transport variant
//! returns a typed `UNSUPPORTED-TRANSPORT` result; it never falls back and
//! never panics. This is a test fixture, not a real transport, and makes no
//! authenticity claim.

use crate::image_record::{ImageError, ImageRecord, PixelFormat, Rotation};
use crate::log_record::{LogError, LogRecord, Severity};
use crate::{Error, Frame, Policy, HEADER_LEN, MAX_PAYLOAD};

/// Typed value naming the unsupported-transport condition.
pub const UNSUPPORTED_TRANSPORT: UnsupportedTransport = UnsupportedTransport::UnsupportedTransport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedTransport {
    /// The requested transport variant has no implementation here.
    UnsupportedTransport,
}

impl core::fmt::Display for UnsupportedTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "UNSUPPORTED-TRANSPORT")
    }
}

/// A transport variant the fake transport may be asked to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// The scripted fake transport (the only supported one).
    Scripted,
    /// Any other variant yields the typed UNSUPPORTED-TRANSPORT result.
    Usb,
}

/// Errors surfaced by the scripted serve path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServeError {
    Log(LogError),
    Image(ImageError),
    Envelope(Error),
    Unsupported(UnsupportedTransport),
}

const LOG_FRAME_CAP: usize = HEADER_LEN + crate::log_record::MAX_LOG_BODY;
const IMG_FRAME_CAP: usize = HEADER_LEN + crate::image_record::IMAGE_HEADER_LEN + 16;

/// The fixed script: 2 log frames then 1 screenshot frame, each a full
/// `DCTL` envelope pre-encoded with the existing codecs. Frame bytes live
/// in these fixed buffers; serving only borrows slices from them.
pub struct Script {
    log_frames: [[u8; LOG_FRAME_CAP]; 2],
    log_lens: [usize; 2],
    img_frame: [u8; IMG_FRAME_CAP],
    img_len: usize,
}

impl Script {
    /// Build the script by encoding valid records through `log_record` and
    /// `image_record`. All frames are correlated with one boot id.
    pub fn new() -> Self {
        let build_id = [7u8; 16];
        let boot_id = [3u8; 16];
        let policy = Policy {
            max_payload: MAX_PAYLOAD,
        };

        let mut log_frames = [[0u8; LOG_FRAME_CAP]; 2];
        let mut log_lens = [0usize; 2];
        for (i, (seq, msg)) in [(1u64, "fake boot"), (2u64, "fake ready")]
            .iter()
            .enumerate()
        {
            let rec = LogRecord::new(build_id, *seq, 1_000 + seq, Severity::Info, "fake", msg)
                .expect("script log record valid");
            log_lens[i] = rec
                .encode_envelope(*seq, &boot_id, &mut log_frames[i], &policy)
                .expect("script log envelope valid");
        }

        // 2x2 RGBA screenshot: stride 8, four pixels of 0x10.
        const PIXELS: [u8; 16] = [0x10; 16];
        let img = ImageRecord::new(
            build_id,
            1,
            2_000,
            2,
            2,
            8,
            PixelFormat::Rgbx8888,
            Rotation::None,
            &PIXELS,
        )
        .expect("script image record valid");
        let mut img_frame = [0u8; IMG_FRAME_CAP];
        let img_len = img
            .encode_into_envelope(1, &boot_id, &mut img_frame, &policy)
            .expect("script image envelope valid");

        Self {
            log_frames,
            log_lens,
            img_frame,
            img_len,
        }
    }
}

impl Default for Script {
    fn default() -> Self {
        Self::new()
    }
}

impl Script {
    /// Half-open byte span `[start, end)` of script frame `i` within the
    /// script's own storage (log frames live in one buffer region, the
    /// screenshot frame in another). Tests use these to prove served slices
    /// are BORROWED from the script buffers, not allocated per frame.
    pub fn frame_span(&self, i: usize) -> Option<(usize, usize)> {
        match i {
            0 => Some((
                self.log_frames[0].as_ptr() as usize - self.span_base(),
                self.log_frames[0].as_ptr() as usize - self.span_base() + self.log_lens[0],
            )),
            1 => Some((
                self.log_frames[1].as_ptr() as usize - self.span_base(),
                self.log_frames[1].as_ptr() as usize - self.span_base() + self.log_lens[1],
            )),
            2 => Some((
                self.img_frame.as_ptr() as usize - self.span_base(),
                self.img_frame.as_ptr() as usize - self.span_base() + self.img_len,
            )),
            _ => None,
        }
    }

    /// Lowest address among the script's frame buffers (span origin).
    pub fn span_base(&self) -> usize {
        let a = self.log_frames.as_ptr() as usize;
        let b = self.img_frame.as_ptr() as usize;
        if a < b {
            a
        } else {
            b
        }
    }

    /// Highest address just past the script's frame buffers.
    pub fn span_end(&self) -> usize {
        let a = self.log_frames.as_ptr() as usize + core::mem::size_of_val(&self.log_frames);
        let b = self.img_frame.as_ptr() as usize + self.img_frame.len();
        if a > b {
            a
        } else {
            b
        }
    }
}

/// Deterministic scripted transport: `serve` hands out BORROWED slices of
/// the script in script order (log, log, screenshot), then None.
pub struct FakeTransport<'a> {
    script: &'a Script,
    next: usize,
}

impl<'a> FakeTransport<'a> {
    /// Construct the scripted transport over a caller-owned script. Served
    /// frames borrow from it — no allocation on the serve path.
    pub fn scripted(script: &'a Script) -> Self {
        Self { script, next: 0 }
    }

    /// Serve the next script frame as a borrowed byte slice, or None when
    /// the script is exhausted. Never allocates or copies.
    pub fn serve(&mut self) -> Option<&'a [u8]> {
        let s = self.script;
        match self.next {
            0 => {
                self.next += 1;
                Some(&s.log_frames[0][..s.log_lens[0]])
            }
            1 => {
                self.next += 1;
                Some(&s.log_frames[1][..s.log_lens[1]])
            }
            2 => {
                self.next += 1;
                Some(&s.img_frame[..s.img_len])
            }
            _ => None,
        }
    }

    /// Parse the next script frame into a borrowed `Frame`, or None.
    pub fn serve_frame(&mut self, policy: &Policy) -> Result<Option<Frame<'a>>, ServeError> {
        match self.serve() {
            None => Ok(None),
            Some(bytes) => Frame::parse(bytes, policy)
                .map(Some)
                .map_err(ServeError::Envelope),
        }
    }
}

/// Ask for a transport variant. `Scripted` is served by a caller-owned
/// script; anything else returns the typed UNSUPPORTED-TRANSPORT result —
/// no fallback, no panic.
pub fn open<'a>(
    kind: TransportKind,
    script: &'a Script,
) -> Result<FakeTransport<'a>, UnsupportedTransport> {
    match kind {
        TransportKind::Scripted => Ok(FakeTransport::scripted(script)),
        TransportKind::Usb => Err(UNSUPPORTED_TRANSPORT),
    }
}
