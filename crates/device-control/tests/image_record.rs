//! Integration tests for the borrowed screenshot-body v1 codec over the
//! accepted control envelope. All wire bytes are literal and hand-written —
//! nothing here depends on the encoder to construct inputs. Every pixel
//! fixture is synthetic host data: NONE of this proves phone capture, fit
//! or delivery.

use device_control::image_record::{
    FramedImage, ImageError, ImageRecord, ImageReject, PixelFormat, Rotation, BYTES_PER_PIXEL,
    IMAGE_HEADER_LEN, MAX_DIMENSION,
};
use device_control::{Error, Frame, Op, Policy, Reject, HEADER_LEN, MAX_PAYLOAD};

// A complete envelope (36-byte header + 48-byte body header + 2 rows x 16
// stride = 32 pixel bytes) with op = Screenshot. Body: 2x3 px at stride 16
// (4 padding bytes per row), format Bgrx8888, rotation Deg90.
const IMG_WIRE: [u8; 132] = build_wire();

const fn build_wire() -> [u8; 132] {
    let mut b = [0u8; 132];
    // envelope header (36 bytes)
    b[0] = b'D';
    b[1] = b'C';
    b[2] = b'T';
    b[3] = b'L';
    b[4] = 1;
    b[5] = 0; // version 1
    b[6] = 2; // op = Screenshot
    b[7] = 0; // reserved
              // correlation id LE = 0x0807060504030201
    let mut i = 0;
    while i < 8 {
        b[8 + i] = (i + 1) as u8;
        i += 1;
    }
    // boot id 0x01..0x10
    i = 0;
    while i < 16 {
        b[16 + i] = (i + 1) as u8;
        i += 1;
    }
    // declared payload = 48 + 48 = 96 LE
    b[32] = 96;
    // body header (48 bytes) at 36..84
    // build id 0xa0..0xaf
    i = 0;
    while i < 16 {
        b[36 + i] = 0xa0 + i as u8;
        i += 1;
    }
    // frame_seq LE = 0x0807060504030201
    i = 0;
    while i < 8 {
        b[52 + i] = (i + 1) as u8;
        i += 1;
    }
    // capture_monotonic_us LE = 0x2a = 42
    b[60] = 42;
    // width = 2 LE
    b[68] = 2;
    // height = 3 LE
    b[72] = 3;
    // stride = 16 bytes LE
    b[76] = 16;
    // pixel format Bgrx8888, rotation Deg90, reserved zero
    b[80] = 1;
    b[81] = 1;
    // pixels: rows of B,G,R,X pattern with distinct bytes + padding 0xf0
    let mut p = 84;
    let mut row = 0;
    while row < 3 {
        let mut col = 0;
        while col < 2 {
            b[p] = (row * 8 + col * 4 + 1) as u8; // B
            b[p + 1] = (row * 8 + col * 4 + 2) as u8; // G
            b[p + 2] = (row * 8 + col * 4 + 3) as u8; // R
            b[p + 3] = 0; // X
            p += 4;
            col += 1;
        }
        b[p] = 0xf0;
        b[p + 1] = 0xf1;
        b[p + 2] = 0xf2;
        b[p + 3] = 0xf3; // padding
        p += 4;
        row += 1;
    }
    b
}

fn body_only_wire() -> Vec<u8> {
    IMG_WIRE[HEADER_LEN..].to_vec()
}

/// Body header bytes with the given geometry, format and rotation, no pixels.
fn header_bytes(width: u32, height: u32, stride: u32, format: u8, rotation: u8) -> Vec<u8> {
    let mut h = vec![0u8; IMAGE_HEADER_LEN];
    for i in 0..16u8 {
        h[i as usize] = 0xa0 + i;
    } // nonzero build id
    for i in 0..8u8 {
        h[16 + i as usize] = i + 1;
    } // nonzero frame_seq
    h[24] = 42; // capture_monotonic_us
    h[32..36].copy_from_slice(&width.to_le_bytes());
    h[36..40].copy_from_slice(&height.to_le_bytes());
    h[40..44].copy_from_slice(&stride.to_le_bytes());
    h[44] = format;
    h[45] = rotation;
    h
}

fn decode_reject(body: &[u8]) -> ImageReject {
    match ImageRecord::decode(body) {
        Err(ImageError::Body(r)) => r,
        other => panic!("expected body rejection, got {other:?}"),
    }
}

#[test]
fn independent_literal_decodes_every_field() {
    let f = Frame::parse(&IMG_WIRE, &Policy::default()).unwrap();
    assert_eq!(f.op(), Op::Screenshot);
    let fi = ImageRecord::from_frame(&f).unwrap();
    let r = fi.record();
    let mut want_build = [0u8; 16];
    for i in 0..16u8 {
        want_build[i as usize] = 0xa0 + i;
    }
    assert_eq!(r.build_id(), &want_build);
    assert_eq!(r.frame_seq(), 0x0807_0605_0403_0201);
    assert_eq!(r.capture_monotonic_us(), 42);
    assert_eq!(r.width(), 2);
    assert_eq!(r.height(), 3);
    assert_eq!(r.stride(), 16);
    assert_eq!(r.pixel_format(), PixelFormat::Bgrx8888);
    assert_eq!(r.rotation(), Rotation::Deg90);
    assert_eq!(r.pixels().len(), 16 * 3);
    // envelope identity is bound, and distinct from body fields
    assert_eq!(fi.correlation_id(), 0x0807_0605_0403_0201);
    let mut want_boot = [0u8; 16];
    for i in 0..16u8 {
        want_boot[i as usize] = i + 1;
    }
    assert_eq!(fi.boot_id(), &want_boot);
    // rotation is metadata: pixels are NOT rotated. Row 0 col 0 literal B=1.
    assert_eq!(&r.pixels()[0..4], &[1, 2, 3, 0]);
}

#[test]
fn padding_bytes_are_preserved_byte_for_byte() {
    let f = Frame::parse(&IMG_WIRE, &Policy::default()).unwrap();
    let fi = ImageRecord::from_frame(&f).unwrap();
    let r = fi.record();
    // row 0 padding is at pixel offset 8..12 in the literal wire
    assert_eq!(&r.pixels()[8..12], &[0xf0, 0xf1, 0xf2, 0xf3]);
    let mut out = vec![0u8; IMG_WIRE.len()];
    let n = r
        .encode_into_envelope(
            0x0807_0605_0403_0201,
            fi_boot(),
            &mut out,
            &Policy::default(),
        )
        .unwrap();
    assert_eq!(n, IMG_WIRE.len());
    assert_eq!(&out[..], &IMG_WIRE[..]); // full byte-for-byte roundtrip
}

fn fi_boot() -> &'static [u8; 16] {
    // boot id from the literal wire: 0x01..0x10
    const B: [u8; 16] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    &B
}

#[test]
fn from_frame_rejects_wrong_op_with_same_valid_body() {
    // Same length and same body bytes, but the envelope op is Logs (1).
    let mut wire = IMG_WIRE;
    wire[6] = 1; // Op::Logs
    let f = Frame::parse(&wire, &Policy::default()).unwrap();
    assert_eq!(f.op(), Op::Logs);
    assert_eq!(
        ImageRecord::from_frame(&f),
        Err(ImageError::Body(ImageReject::WrongOp))
    );
}

#[test]
fn zero_and_out_of_range_dims_rejected() {
    let b = header_bytes(0, 3, 16, 1, 0); // width 0
    assert_eq!(decode_reject(&b), ImageReject::WidthOutOfRange);
    let b = header_bytes(4097, 1, 4097 * 4, 1, 0); // width over max
    assert_eq!(decode_reject(&b), ImageReject::WidthOutOfRange);
    let b = header_bytes(2, 0, 16, 1, 0); // height 0
    assert_eq!(decode_reject(&b), ImageReject::HeightOutOfRange);
    let b = header_bytes(2, MAX_DIMENSION + 1, 4096 * 4, 1, 0); // height over max
    assert_eq!(decode_reject(&b), ImageReject::HeightOutOfRange);
    // max dimensions themselves are accepted by the bounds (then need pixels)
    // 4096x4096 at stride 16384 is 64 Mi of pixels: over the core cap, and
    // that check happens before any slicing.
    let b = header_bytes(MAX_DIMENSION, MAX_DIMENSION, MAX_DIMENSION * 4, 1, 0);
    assert_eq!(decode_reject(&b), ImageReject::BodyOverHardCap);
}

#[test]
fn overflow_dims_and_stride_products_rejected_before_slicing() {
    // The pixel count itself fits u32, but adding the header would not.
    // Reject before pointer-width narrowing or header addition.
    let b = header_bytes(1, 1, u32::MAX, 1, 0);
    assert_eq!(decode_reject(&b), ImageReject::BodyOverHardCap);
    // u32-max width: out of range on the dimension bound.
    let b = header_bytes(u32::MAX, u32::MAX, u32::MAX, 1, 0);
    assert_eq!(decode_reject(&b), ImageReject::WidthOutOfRange);
    // in-range dims whose stride*height exceeds the 16 MiB core cap.
    let b = header_bytes(4096, 4096, 4096 * 4, 1, 0); // 64 Mi of pixels
    assert_eq!(decode_reject(&b), ImageReject::BodyOverHardCap);
    // huge stride with valid dims: over cap, not a wrap.
    let b = header_bytes(2, 4096, u32::MAX, 1, 0);
    assert_eq!(decode_reject(&b), ImageReject::BodyOverHardCap);
}

#[test]
fn short_stride_below_width_times_four_rejected() {
    // width 5 needs stride >= 20; 19 is rejected.
    let b = header_bytes(5, 1, 19, 1, 0);
    assert_eq!(decode_reject(&b), ImageReject::StrideTooSmall);
    // exactly width*4 is fine (then length check).
    let mut b = header_bytes(5, 1, 20, 1, 0);
    b.extend_from_slice(&[7u8; 20]);
    assert!(ImageRecord::decode(&b).is_ok());
}

#[test]
fn unknown_format_rotation_and_reserved_rejected() {
    for fmt in [0u8, 3, 4, 255] {
        let mut b = header_bytes(1, 1, 4, fmt, 0);
        b.extend_from_slice(&[9u8; 4]);
        assert_eq!(
            decode_reject(&b),
            ImageReject::UnknownPixelFormat,
            "fmt {fmt}"
        );
    }
    for rot in [4u8, 5, 255] {
        let mut b = header_bytes(1, 1, 4, 2, rot);
        b.extend_from_slice(&[9u8; 4]);
        assert_eq!(decode_reject(&b), ImageReject::UnknownRotation, "rot {rot}");
    }
    // both accepted formats and all four rotations decode.
    for fmt in [1u8, 2] {
        for rot in [0u8, 1, 2, 3] {
            let mut b = header_bytes(1, 1, 4, fmt, rot);
            b.extend_from_slice(&[9u8; 4]);
            let r = ImageRecord::decode(&b).unwrap();
            assert_eq!(r.pixels(), &[9u8; 4]);
        }
    }
    // reserved bytes must be zero, individually.
    let mut b = header_bytes(1, 1, 4, 1, 0);
    b[46] = 1;
    b.extend_from_slice(&[9u8; 4]);
    assert_eq!(decode_reject(&b), ImageReject::ReservedNonZero);
    let mut b = header_bytes(1, 1, 4, 1, 0);
    b[47] = 0xff;
    b.extend_from_slice(&[9u8; 4]);
    assert_eq!(decode_reject(&b), ImageReject::ReservedNonZero);
}

#[test]
fn zero_build_id_and_zero_frame_seq_rejected() {
    let mut b = header_bytes(1, 1, 4, 1, 0);
    for i in 0..16 {
        b[i] = 0;
    }
    b.extend_from_slice(&[9u8; 4]);
    assert_eq!(decode_reject(&b), ImageReject::ZeroBuildId);
    let mut b = header_bytes(1, 1, 4, 1, 0);
    for i in 16..24 {
        b[i] = 0;
    }
    b.extend_from_slice(&[9u8; 4]);
    assert_eq!(decode_reject(&b), ImageReject::ZeroFrameSeq);
    // capture_monotonic_us == 0 is VALID.
    let mut b = header_bytes(1, 1, 4, 1, 0);
    for i in 24..32 {
        b[i] = 0;
    }
    b.extend_from_slice(&[9u8; 4]);
    assert_eq!(ImageRecord::decode(&b).unwrap().capture_monotonic_us(), 0);
}

#[test]
fn truncated_trailing_and_wrong_length_bodies_rejected() {
    let body = body_only_wire(); // 80 bytes: header + 32 pixels
    assert!(ImageRecord::decode(&body).is_ok());
    // body shorter than the fixed header
    assert_eq!(
        ImageRecord::decode(&body[..IMAGE_HEADER_LEN - 1]),
        Err(ImageError::Body(ImageReject::BodyTooShortForHeader))
    );
    // truncated pixels
    assert_eq!(
        ImageRecord::decode(&body[..body.len() - 1]),
        Err(ImageError::Body(ImageReject::BodyLengthMismatch))
    );
    // trailing pixel bytes
    let mut extra = body.clone();
    extra.push(0xaa);
    assert_eq!(
        ImageRecord::decode(&extra),
        Err(ImageError::Body(ImageReject::BodyLengthMismatch))
    );
    // envelope-level truncation and trailing stay in the core error type
    let mut short_env = IMG_WIRE.to_vec();
    short_env.pop();
    assert_eq!(
        Frame::parse(&short_env, &Policy::default()),
        Err(Error::Malformed(Reject::TruncatedPayload))
    );
    let mut long_env = IMG_WIRE.to_vec();
    long_env.push(0);
    assert_eq!(
        Frame::parse(&long_env, &Policy::default()),
        Err(Error::Malformed(Reject::TrailingBytes))
    );
}

#[test]
fn pixels_borrow_the_input_buffer_no_copy() {
    // Pointer identity: the decoded pixel slice must start exactly where the
    // body header ended in the ORIGINAL buffer — nothing was copied.
    let body = body_only_wire();
    let r = ImageRecord::decode(&body).unwrap();
    assert_eq!(r.pixels().as_ptr(), body[IMAGE_HEADER_LEN..].as_ptr());
    assert_eq!(r.pixels().len(), 48);

    // And through the envelope: pixels borrow the caller's whole wire.
    let f = Frame::parse(&IMG_WIRE, &Policy::default()).unwrap();
    let fi = ImageRecord::from_frame(&f).unwrap();
    assert_eq!(
        fi.record().pixels().as_ptr(),
        IMG_WIRE[HEADER_LEN + IMAGE_HEADER_LEN..].as_ptr()
    );
}

#[test]
fn encode_rejections_leave_output_byte_for_byte_unchanged() {
    let sentinel: Vec<u8> = (0u8..=255).cycle().take(4096).collect();

    // A record whose declared geometry does not match its pixel length is
    // constructed only via decode; here we corrupt a decoded body instead and
    // re-encode the valid record into too-small or poisoned outputs.
    let body = body_only_wire();
    let r = ImageRecord::decode(&body).unwrap();

    // 1. Output too small: error names required/capacity, output untouched.
    let mut out = sentinel.clone();
    let cut = HEADER_LEN + IMAGE_HEADER_LEN + 47; // one pixel byte short
    let err = r
        .encode_into_envelope(1, fi_boot(), &mut out[..cut], &Policy::default())
        .unwrap_err();
    assert_eq!(
        err,
        ImageError::Envelope(Error::OutputTooSmall {
            required: IMG_WIRE.len(),
            capacity: cut
        })
    );
    assert_eq!(&out[..], &sentinel[..]);

    // 2. Zero correlation id and zero boot id are envelope rejections; the
    //    output is untouched either way.
    let mut out = sentinel.clone();
    assert_eq!(
        r.encode_into_envelope(0, fi_boot(), &mut out, &Policy::default()),
        Err(ImageError::Envelope(Error::Malformed(
            Reject::ZeroCorrelationId
        )))
    );
    assert_eq!(&out[..], &sentinel[..]);
    let mut out = sentinel.clone();
    assert_eq!(
        r.encode_into_envelope(1, &[0u8; 16], &mut out, &Policy::default()),
        Err(ImageError::Envelope(Error::Malformed(Reject::ZeroBootId)))
    );
    assert_eq!(&out[..], &sentinel[..]);

    // 3. A tightened policy that the body exceeds is a per-rejection with no
    //    partial write.
    let mut out = sentinel.clone();
    assert_eq!(
        r.encode_into_envelope(1, fi_boot(), &mut out, &Policy { max_payload: 47 }),
        Err(ImageError::Envelope(Error::Malformed(
            Reject::DeclaredLengthOverPolicyCap
        )))
    );
    assert_eq!(&out[..], &sentinel[..]);

    // 4. A policy looser than the hard cap is InvalidPolicy, not clamped.
    let mut out = sentinel.clone();
    assert_eq!(
        r.encode_into_envelope(
            1,
            fi_boot(),
            &mut out,
            &Policy {
                max_payload: MAX_PAYLOAD + 1
            }
        ),
        Err(ImageError::Envelope(Error::InvalidPolicy))
    );
    assert_eq!(&out[..], &sentinel[..]);

    // 5. Body-only encode into a too-small buffer leaves it untouched.
    let mut out = sentinel.clone();
    assert_eq!(
        r.encode_body(&mut out[..IMAGE_HEADER_LEN + 47]),
        Err(ImageError::Envelope(Error::OutputTooSmall {
            required: IMAGE_HEADER_LEN + 48,
            capacity: IMAGE_HEADER_LEN + 47
        }))
    );
    assert_eq!(&out[..], &sentinel[..]);

    // 6. A record built by `new` with mismatched pixel length is rejected at
    //    construction; nothing to encode.
    let bad = ImageRecord::new(
        [0xa1; 16],
        7,
        0,
        5,
        1,
        20,
        PixelFormat::Rgbx8888,
        Rotation::None,
        &[3u8; 19],
    );
    assert_eq!(
        bad.unwrap_err(),
        ImageError::Body(ImageReject::BodyLengthMismatch)
    );
}

#[test]
fn successful_encode_writes_exact_bytes_and_tail_unchanged() {
    let body = body_only_wire();
    let r = ImageRecord::decode(&body).unwrap();
    let sentinel: Vec<u8> = (0u8..=255).cycle().take(4096).collect();

    // Output exactly the right size; everything after `written` untouched.
    let mut out = sentinel.clone();
    let n = r
        .encode_into_envelope(
            0x0807_0605_0403_0201,
            fi_boot(),
            &mut out,
            &Policy::default(),
        )
        .unwrap();
    assert_eq!(n, IMG_WIRE.len());
    assert_eq!(&out[..n], &IMG_WIRE[..]);
    assert_eq!(&out[n..], &sentinel[n..]);

    // Body-only encode roundtrips the literal body exactly.
    let mut out = sentinel.clone();
    let n = r.encode_body(&mut out).unwrap();
    assert_eq!(n, IMAGE_HEADER_LEN + 48);
    assert_eq!(&out[..n], &body[..]);
    assert_eq!(&out[n..], &sentinel[n..]);

    // The re-encoded envelope parses back to an equal record.
    let mut env = vec![0u8; IMG_WIRE.len()];
    let n = r
        .encode_into_envelope(
            0x0807_0605_0403_0201,
            fi_boot(),
            &mut env,
            &Policy::default(),
        )
        .unwrap();
    let f = Frame::parse(&env[..n], &Policy::default()).unwrap();
    let fi: FramedImage = ImageRecord::from_frame(&f).unwrap();
    assert_eq!(fi.record(), &r);
}

#[test]
fn real_sized_synthetic_layout_probe_is_layout_proof_only() {
    // 1080x2280 at stride 4320 (= 1080*4): a REAL-SIZED host vector proving
    // the layout arithmetic roundtrips at phone-scale. The pixel bytes are
    // SYNTHETIC — this is not, and must never be cited as, phone capture,
    // fit, or delivery evidence.
    let (w, h, stride) = (1080u32, 2280u32, 4320u32);
    let plen = (stride * h) as usize; // 9_849_600 < 16 MiB
    assert!(plen + IMAGE_HEADER_LEN <= MAX_PAYLOAD as usize);
    let pixels: Vec<u8> = (0u8..=255).cycle().take(plen).collect();
    let r = ImageRecord::new(
        [0x5c; 16],
        9,
        1_700_000_000_000_000,
        w,
        h,
        stride,
        PixelFormat::Bgrx8888,
        Rotation::Deg270,
        &pixels,
    )
    .unwrap();
    assert_eq!(r.encoded_len().unwrap(), IMAGE_HEADER_LEN + plen);

    let mut out = vec![0u8; HEADER_LEN + IMAGE_HEADER_LEN + plen];
    let n = r
        .encode_into_envelope(5, &[0xbb; 16], &mut out, &Policy::default())
        .unwrap();
    assert_eq!(n, out.len());

    // Decode straight from the wire: same metadata, borrowed pixels intact.
    let f = Frame::parse(&out, &Policy::default()).unwrap();
    let fi = ImageRecord::from_frame(&f).unwrap();
    let d = fi.record();
    assert_eq!((d.width(), d.height(), d.stride()), (w, h, stride));
    assert_eq!(d.rotation(), Rotation::Deg270);
    assert_eq!(d.pixels().len(), plen);
    assert_eq!(d.pixels(), &pixels[..]);
    // spot-check a padded-row boundary is not disturbed: stride == width*4
    // here, so every 4 bytes are one pixel; BYTES_PER_PIXEL sanity:
    assert_eq!(BYTES_PER_PIXEL * w, stride);
}
