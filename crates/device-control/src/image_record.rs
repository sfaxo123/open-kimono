//! Borrowed raw screenshot BODY codec for the accepted control-envelope v1.
//!
//! This module decodes and encodes ONE screenshot-body version 1 payload
//! carried by an already-validated [`Frame`] with `op == Op::Screenshot`.
//! It is a proposed, frozen body schema for this partial 0.2.1 slice — it is
//! NOT screen capture or rendering, it is NOT compatible with tracejournal
//! or any existing device transport, and it makes no claim that any live
//! device or phone emits these bodies. Decoded pixel bytes are synthetic
//! data unless a real capture produced them elsewhere; this codec cannot
//! tell the difference and never claims to.
//!
//! Exact screenshot-body v1 layout (little-endian), fixed 48-byte header:
//! - bytes 0..16   build id, must be nonzero;
//! - bytes 16..24  u64 frame sequence, must be nonzero;
//! - bytes 24..32  u64 capture monotonic microseconds (0 is valid);
//! - bytes 32..36  u32 width, must be 1..=4096;
//! - bytes 36..40  u32 height, must be 1..=4096;
//! - bytes 40..44  u32 stride in BYTES (not pixels), must be >= width*4;
//! - byte  44      pixel format: 1 = Bgrx8888, 2 = Rgbx8888;
//! - byte  45      rotation: 0..=3 meaning 0/90/180/270 degrees of metadata.
//!                   Pixels are NEVER rotated silently — the value is kept;
//! - bytes 46..48  two reserved bytes, must be zero;
//! - then exactly `stride * height` pixel bytes, row padding included and
//!   preserved byte-for-byte.
//!
//! The whole body (header + pixels) must not exceed the core envelope hard
//! cap [`MAX_PAYLOAD`] (16 MiB); every product and conversion is checked
//! before any slicing or conversion. Parsing never allocates and copies no
//! pixels: the pixel slice borrows the caller's buffer. Encoding writes only
//! into caller-provided output and validates ALL metadata, length products
//! and output capacity BEFORE any write, so a rejected encode leaves the
//! output byte-for-byte unchanged. There is no hidden 16 MiB stack or heap
//! staging buffer anywhere in this module. Decoding also copies no pixels:
//! `pixels()` borrows the caller's buffer at its original address.
//!
//! Deliberately NOT here: PNG or any pixel conversion, authentication,
//! freshness/staleness policy, capture or rendering, runtime concerns, and
//! any change to the core error type. [`ImageError`] wraps envelope-level
//! failures through `Envelope(crate::Error)`.

use crate::{Error, Frame, Op, MAX_PAYLOAD};
use core::fmt;

/// Screenshot-body schema version implemented by this module.
pub const IMAGE_BODY_VERSION: u8 = 1;
/// Fixed size of the screenshot-body v1 header.
pub const IMAGE_HEADER_LEN: usize = 48;
/// Maximum width and height in pixels (both dimensions are bounded).
pub const MAX_DIMENSION: u32 = 4096;
/// Bytes per pixel for both accepted formats (BGRX8888, RGBX8888).
pub const BYTES_PER_PIXEL: u32 = 4;

/// Accepted raw pixel formats. The wire value is validated before
/// construction, so an unknown format can never become a `PixelFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgrx8888 = 1,
    Rgbx8888 = 2,
}

impl PixelFormat {
    fn from_u8(b: u8) -> Option<PixelFormat> {
        match b {
            1 => Some(PixelFormat::Bgrx8888),
            2 => Some(PixelFormat::Rgbx8888),
            _ => None,
        }
    }

    fn to_u8(self) -> u8 {
        self as u8
    }
}

/// Rotation metadata: 0/90/180/270 degrees. This module records and reports
/// it; it never rotates pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    None = 0,
    Deg90 = 1,
    Deg180 = 2,
    Deg270 = 3,
}

impl Rotation {
    fn from_u8(b: u8) -> Option<Rotation> {
        match b {
            0 => Some(Rotation::None),
            1 => Some(Rotation::Deg90),
            2 => Some(Rotation::Deg180),
            3 => Some(Rotation::Deg270),
            _ => None,
        }
    }

    fn to_u8(self) -> u8 {
        self as u8
    }
}
/// Body-level rejections. Envelope-level rejections (bad magic, wrong op at
/// the frame layer, trailing bytes, ...) come from [`Frame::parse`] /
/// [`Frame::encode`] and are surfaced as [`ImageError::Envelope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageReject {
    /// `Frame` op was not `Op::Screenshot`.
    WrongOp,
    /// Body shorter than the 48-byte fixed header.
    BodyTooShortForHeader,
    ZeroBuildId,
    ZeroFrameSeq,
    WidthOutOfRange,
    HeightOutOfRange,
    /// Declared stride below `width * BYTES_PER_PIXEL`.
    StrideTooSmall,
    UnknownPixelFormat,
    UnknownRotation,
    ReservedNonZero,
    /// `stride * height` overflowed, or header + pixel bytes exceed the
    /// core 16 MiB hard cap.
    BodyOverHardCap,
    /// Actual body length does not equal header + stride*height exactly.
    BodyLengthMismatch,
}

/// The single error type for this module: body rejections stay in the image
/// namespace; envelope failures are wrapped from the core [`Error`], which
/// this module never edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageError {
    Body(ImageReject),
    Envelope(Error),
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImageError::Body(r) => write!(f, "malformed screenshot body: {r:?}"),
            ImageError::Envelope(e) => write!(f, "envelope error: {e}"),
        }
    }
}

/// One fully validated screenshot body. Fields are private and only
/// reachable through already-validated accessors; the pixels borrow the
/// decoded buffer — no pixel bytes are ever copied during parse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageRecord<'a> {
    build_id: [u8; 16],
    frame_seq: u64,
    capture_monotonic_us: u64,
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: PixelFormat,
    rotation: Rotation,
    pixels: &'a [u8],
}

/// A decoded [`ImageRecord`] bound to the envelope it arrived in: the
/// record plus the validated correlation id and boot id of that envelope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramedImage<'a> {
    record: ImageRecord<'a>,
    correlation_id: u64,
    boot_id: [u8; 16],
}

impl<'a> FramedImage<'a> {
    pub fn record(&self) -> &ImageRecord<'a> {
        &self.record
    }
    /// The envelope's validated nonzero correlation id, distinct from the
    /// body's own frame sequence.
    pub fn correlation_id(&self) -> u64 {
        self.correlation_id
    }
    /// The envelope's validated nonzero 16-byte boot id, distinct from the
    /// body's build id.
    pub fn boot_id(&self) -> &[u8; 16] {
        &self.boot_id
    }
}

/// Checked `stride * height` pixel length, or the hard-cap rejection. The
/// product is computed in u64 so it cannot overflow on any target, and the
/// header-inclusive total is compared against the core 16 MiB cap.
fn pixel_len(stride: u32, height: u32) -> Result<usize, ImageError> {
    let prod = u64::from(stride) * u64::from(height);
    // Apply the wire cap before narrowing to the target's pointer width.
    // In particular, u32::MAX stride on a 32-bit target must not overflow
    // when the header is added to an otherwise representable pixel length.
    if prod + IMAGE_HEADER_LEN as u64 > u64::from(MAX_PAYLOAD) {
        return Err(ImageError::Body(ImageReject::BodyOverHardCap));
    }
    usize::try_from(prod).map_err(|_| ImageError::Body(ImageReject::BodyOverHardCap))
}

/// Checked `width * BYTES_PER_PIXEL` in the wire's u32 representation.
fn min_stride(width: u32) -> Result<u32, ImageError> {
    match width.checked_mul(BYTES_PER_PIXEL) {
        Some(n) => Ok(n),
        None => Err(ImageError::Body(ImageReject::StrideTooSmall)),
    }
}

/// Shared metadata+length validation for `new`, `encode_body` and
/// `encoded_len`. Rejects on the first violation; checks nothing else.
fn check_fields(r: &ImageRecord<'_>) -> Result<usize, ImageError> {
    if r.build_id.iter().all(|&x| x == 0) {
        return Err(ImageError::Body(ImageReject::ZeroBuildId));
    }
    if r.frame_seq == 0 {
        return Err(ImageError::Body(ImageReject::ZeroFrameSeq));
    }
    if r.width < 1 || r.width > MAX_DIMENSION {
        return Err(ImageError::Body(ImageReject::WidthOutOfRange));
    }
    if r.height < 1 || r.height > MAX_DIMENSION {
        return Err(ImageError::Body(ImageReject::HeightOutOfRange));
    }
    if r.stride < min_stride(r.width)? {
        return Err(ImageError::Body(ImageReject::StrideTooSmall));
    }
    let plen = pixel_len(r.stride, r.height)?;
    if r.pixels.len() != plen {
        return Err(ImageError::Body(ImageReject::BodyLengthMismatch));
    }
    Ok(plen)
}

impl<'a> ImageRecord<'a> {
    /// Construct a producer record from validated metadata plus the exact
    /// pixel bytes (`pixels.len()` must equal `stride * height`). Pixels are
    /// borrowed, never copied. A record is not authenticated or envelope-bound.
    pub fn new(
        build_id: [u8; 16],
        frame_seq: u64,
        capture_monotonic_us: u64,
        width: u32,
        height: u32,
        stride: u32,
        pixel_format: PixelFormat,
        rotation: Rotation,
        pixels: &'a [u8],
    ) -> Result<Self, ImageError> {
        let candidate = Self {
            build_id,
            frame_seq,
            capture_monotonic_us,
            width,
            height,
            stride,
            pixel_format,
            rotation,
            pixels,
        };
        check_fields(&candidate)?;
        Ok(candidate)
    }

    /// Decode a screenshot body from an already-parsed envelope, keeping the
    /// envelope's correlation id and boot id bound to the record. The frame
    /// must carry `op == Op::Screenshot`. Every bound is checked BEFORE any
    /// slicing; pixels borrow the original input buffer — no allocation,
    /// no copy.
    pub fn from_frame(frame: &Frame<'a>) -> Result<FramedImage<'a>, ImageError> {
        if frame.op() != Op::Screenshot {
            return Err(ImageError::Body(ImageReject::WrongOp));
        }
        let record = Self::decode(frame.payload())?;
        Ok(FramedImage {
            record,
            correlation_id: frame.correlation_id(),
            boot_id: *frame.boot_id(),
        })
    }

    /// Decode a bare screenshot body (the payload bytes of a `Screenshot`
    /// frame). The result is explicitly NOT bound to any envelope. Use
    /// [`ImageRecord::from_frame`] when envelope identity matters.
    pub fn decode(body: &'a [u8]) -> Result<ImageRecord<'a>, ImageError> {
        if body.len() < IMAGE_HEADER_LEN {
            return Err(ImageError::Body(ImageReject::BodyTooShortForHeader));
        }
        let build_id: [u8; 16] = body[0..16].try_into().unwrap();
        if build_id.iter().all(|&x| x == 0) {
            return Err(ImageError::Body(ImageReject::ZeroBuildId));
        }
        let frame_seq = u64::from_le_bytes(body[16..24].try_into().unwrap());
        if frame_seq == 0 {
            return Err(ImageError::Body(ImageReject::ZeroFrameSeq));
        }
        let capture_monotonic_us = u64::from_le_bytes(body[24..32].try_into().unwrap());
        let width = u32::from_le_bytes(body[32..36].try_into().unwrap());
        if width < 1 || width > MAX_DIMENSION {
            return Err(ImageError::Body(ImageReject::WidthOutOfRange));
        }
        let height = u32::from_le_bytes(body[36..40].try_into().unwrap());
        if height < 1 || height > MAX_DIMENSION {
            return Err(ImageError::Body(ImageReject::HeightOutOfRange));
        }
        let stride = u32::from_le_bytes(body[40..44].try_into().unwrap());
        if stride < min_stride(width)? {
            return Err(ImageError::Body(ImageReject::StrideTooSmall));
        }
        let pixel_format = PixelFormat::from_u8(body[44])
            .ok_or(ImageError::Body(ImageReject::UnknownPixelFormat))?;
        let rotation =
            Rotation::from_u8(body[45]).ok_or(ImageError::Body(ImageReject::UnknownRotation))?;
        if body[46] != 0 || body[47] != 0 {
            return Err(ImageError::Body(ImageReject::ReservedNonZero));
        }
        let plen = pixel_len(stride, height)?;
        let total = IMAGE_HEADER_LEN + plen;
        if body.len() != total {
            return Err(ImageError::Body(ImageReject::BodyLengthMismatch));
        }
        Ok(Self {
            build_id,
            frame_seq,
            capture_monotonic_us,
            width,
            height,
            stride,
            pixel_format,
            rotation,
            pixels: &body[IMAGE_HEADER_LEN..total],
        })
    }

    pub fn build_id(&self) -> &[u8; 16] {
        &self.build_id
    }
    pub fn frame_seq(&self) -> u64 {
        self.frame_seq
    }
    pub fn capture_monotonic_us(&self) -> u64 {
        self.capture_monotonic_us
    }
    pub fn width(&self) -> u32 {
        self.width
    }
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Row stride in BYTES (>= width*4); padding bytes are preserved.
    pub fn stride(&self) -> u32 {
        self.stride
    }
    pub fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }
    /// Rotation METADATA only; pixels were never rotated by this codec.
    pub fn rotation(&self) -> Rotation {
        self.rotation
    }
    /// Borrowed pixel bytes: exactly `stride * height`, padding included.
    pub fn pixels(&self) -> &'a [u8] {
        self.pixels
    }

    /// Encode the body into caller-provided `output`. All metadata, every
    /// length product and the output capacity are validated BEFORE any byte
    /// is written, so a rejected encode leaves `output` byte-for-byte
    /// unchanged. Returns the number of bytes written. No hidden staging
    /// buffer: pixels go straight from `self.pixels` into `output`.
    pub fn encode_body(&self, output: &mut [u8]) -> Result<usize, ImageError> {
        let plen = check_fields(self)?;
        let total = IMAGE_HEADER_LEN + plen;
        if output.len() < total {
            return Err(ImageError::Envelope(Error::OutputTooSmall {
                required: total,
                capacity: output.len(),
            }));
        }
        // All validation passed; only now write.
        output[0..16].copy_from_slice(&self.build_id);
        output[16..24].copy_from_slice(&self.frame_seq.to_le_bytes());
        output[24..32].copy_from_slice(&self.capture_monotonic_us.to_le_bytes());
        output[32..36].copy_from_slice(&self.width.to_le_bytes());
        output[36..40].copy_from_slice(&self.height.to_le_bytes());
        output[40..44].copy_from_slice(&self.stride.to_le_bytes());
        output[44] = self.pixel_format.to_u8();
        output[45] = self.rotation.to_u8();
        output[46] = 0;
        output[47] = 0;
        output[IMAGE_HEADER_LEN..total].copy_from_slice(self.pixels);
        Ok(total)
    }

    /// Body length this record encodes to, or the corresponding rejection.
    pub fn encoded_len(&self) -> Result<usize, ImageError> {
        Ok(IMAGE_HEADER_LEN + check_fields(self)?)
    }

    /// Encode the full envelope (36-byte frame header + this body) directly
    /// into caller-provided `output` with no intermediate buffer: capacity
    /// and all metadata are validated first, then the frame header, the body
    /// header and the pixels are written in place. Returns total bytes
    /// written; a rejection leaves `output` byte-for-byte unchanged.
    pub fn encode_into_envelope(
        &self,
        correlation_id: u64,
        boot_id: &[u8; 16],
        output: &mut [u8],
        policy: &crate::Policy,
    ) -> Result<usize, ImageError> {
        let plen = check_fields(self)?;
        if policy.max_payload > MAX_PAYLOAD {
            return Err(ImageError::Envelope(Error::InvalidPolicy));
        }
        if correlation_id == 0 {
            return Err(ImageError::Envelope(Error::Malformed(
                crate::Reject::ZeroCorrelationId,
            )));
        }
        if boot_id.iter().all(|&x| x == 0) {
            return Err(ImageError::Envelope(Error::Malformed(
                crate::Reject::ZeroBootId,
            )));
        }
        let body_len = IMAGE_HEADER_LEN + plen;
        let body_len32 =
            u32::try_from(body_len).map_err(|_| ImageError::Body(ImageReject::BodyOverHardCap))?;
        if body_len32 > policy.max_payload {
            return Err(ImageError::Envelope(Error::Malformed(
                crate::Reject::DeclaredLengthOverPolicyCap,
            )));
        }
        let total = crate::HEADER_LEN + body_len;
        if output.len() < total {
            return Err(ImageError::Envelope(Error::OutputTooSmall {
                required: total,
                capacity: output.len(),
            }));
        }
        // All validation passed; only now write.
        output[0..4].copy_from_slice(crate::MAGIC);
        output[4..6].copy_from_slice(&crate::VERSION.to_le_bytes());
        output[6] = Op::Screenshot as u8;
        output[7] = 0;
        output[8..16].copy_from_slice(&correlation_id.to_le_bytes());
        output[16..32].copy_from_slice(boot_id);
        output[32..36].copy_from_slice(&body_len32.to_le_bytes());
        let body_end = crate::HEADER_LEN + body_len;
        self.encode_body(&mut output[crate::HEADER_LEN..body_end])?;
        Ok(total)
    }
}

impl fmt::Display for ImageRecord<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "screenshot v{IMAGE_BODY_VERSION} frame {} {}x{} stride {} fmt {:?} rot {:?}",
            self.frame_seq, self.width, self.height, self.stride, self.pixel_format, self.rotation
        )
    }
}
