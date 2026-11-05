//! Integration tests for the borrowed log-body v1 codec over the accepted
//! control envelope. All wire bytes are literal and hand-written — nothing
//! here depends on the encoder to construct inputs.

use device_control::log_record::{
    FramedLog, LogError, LogRecord, LogReject, Severity, LOG_HEADER_LEN, MAX_LOG_BODY,
    MAX_MESSAGE_BYTES,
};
use device_control::{Error, Frame, Op, Policy, Reject, HEADER_LEN};

// A complete envelope (36-byte header + 51-byte log body) with op = Logs.
// Body: build id 0x10..0x1f, seq 0x0807060504030201 LE, monotonic_us 7 LE,
// severity 3 (Warning), subsystem len 4 ("klog"), message len 9 LE,
// subsystem "klog", message "hi\n\0there" (h i \n \0 t h e r e).
const LOG_WIRE: [u8; 87] = [
    // envelope header (36 bytes)
    0x44, 0x43, 0x54, 0x4c, // "DCTL"
    0x01, 0x00, // version 1
    0x01, // op = Logs
    0x00, // reserved
    0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // correlation id LE
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, // boot id
    0x33, 0x00, 0x00, 0x00, // declared payload = 51
    // log body header (38 bytes)
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
    0x1f, // build id
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // sequence LE
    0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // monotonic_us = 7
    0x03, // severity Warning
    0x04, // subsystem bytes = 4
    0x09, 0x00, 0x00, 0x00, // message bytes = 9
    // subsystem "klog"
    0x6b, 0x6c, 0x6f, 0x67, // message "hi\n\0there"
    0x68, 0x69, 0x0a, 0x00, 0x74, 0x68, 0x65, 0x72, 0x65,
];

fn body_only_wire() -> [u8; 51] {
    LOG_WIRE[HEADER_LEN..]
        .try_into()
        .expect("literal slice length")
}

#[test]
fn independent_literal_decodes_every_field() {
    let f = Frame::parse(&LOG_WIRE, &Policy::default()).unwrap();
    assert_eq!(f.op(), Op::Logs);
    let fl = LogRecord::from_frame(&f).unwrap();
    let r = fl.record();
    assert_eq!(
        r.build_id(),
        &[
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x1f
        ]
    );
    assert_eq!(r.sequence(), 0x0807_0605_0403_0201);
    assert_eq!(r.monotonic_us(), 7);
    assert_eq!(r.severity(), Severity::Warning);
    assert_eq!(r.subsystem(), "klog");
    // Message roundtrips losslessly: newline AND embedded NUL survive.
    assert_eq!(r.message(), "hi\n\0there");
    assert_eq!(r.message().as_bytes(), b"hi\n\0there");
}

#[test]
fn framed_log_retains_envelope_correlation_and_boot_identity() {
    let f = Frame::parse(&LOG_WIRE, &Policy::default()).unwrap();
    let fl = LogRecord::from_frame(&f).unwrap();
    // Envelope identity is distinct from body build id / sequence.
    assert_eq!(fl.correlation_id(), 0x0102_0304_0506_0708);
    assert_ne!(fl.correlation_id(), fl.record().sequence());
    assert_eq!(
        fl.boot_id(),
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    assert_ne!(fl.boot_id(), fl.record().build_id());
}

#[test]
fn bare_decode_is_unbound_but_framed_log_matches_from_frame() {
    let wire = body_only_wire();
    let bare = LogRecord::decode(&wire).unwrap();
    let f = Frame::parse(&LOG_WIRE, &Policy::default()).unwrap();
    let fl = LogRecord::from_frame(&f).unwrap();
    // The record content agrees; only the FramedLog carries envelope ids.
    assert_eq!(*fl.record(), bare);
    let _: &FramedLog = &fl;
}

#[test]
fn decode_borrows_the_original_buffer_not_a_copy() {
    let f = Frame::parse(&LOG_WIRE, &Policy::default()).unwrap();
    let r = LogRecord::from_frame(&f).unwrap();
    let r = r.record();
    assert_eq!(r.subsystem().as_ptr(), LOG_WIRE[HEADER_LEN + 38..].as_ptr());
    assert_eq!(
        r.message().as_ptr(),
        LOG_WIRE[HEADER_LEN + 38 + 4..].as_ptr()
    );
}

#[test]
fn wrong_op_rejected_even_with_equal_length_body() {
    // Same total length, only the op byte changed (Logs=1 -> Command=3).
    let mut wire = LOG_WIRE;
    wire[6] = 0x03;
    let f = Frame::parse(&wire, &Policy::default()).unwrap();
    assert_eq!(f.op(), Op::Command);
    assert_eq!(
        LogRecord::from_frame(&f),
        Err(LogError::Body(LogReject::WrongOp))
    );
}

#[test]
fn body_shorter_than_header_rejected() {
    let body = &body_only_wire()[..LOG_HEADER_LEN - 1];
    assert_eq!(
        LogRecord::decode(body),
        Err(LogError::Body(LogReject::BodyTooShortForHeader))
    );
}

#[test]
fn actual_body_over_hard_cap_rejected_early() {
    // Body longer than MAX_LOG_BODY (4198) must be rejected on length alone,
    // regardless of declared fields. Build one with a plausible header.
    let mut w = vec![0u8; MAX_LOG_BODY + 1];
    for i in 0..16 {
        w[i] = 0xaa;
    }
    w[16..24].copy_from_slice(&1u64.to_le_bytes());
    w[32] = 0;
    w[33] = 1;
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::BodyOverHardCap))
    );
}

#[test]
fn truncated_strings_rejected_before_decoding() {
    // Valid 38-byte header with declared subsystem 4 + message 9, but the
    // string bytes are entirely absent: bounds check must reject.
    let w = &body_only_wire()[..LOG_HEADER_LEN];
    assert_eq!(
        LogRecord::decode(w),
        Err(LogError::Body(LogReject::BodyLengthMismatch))
    );
    // Partial strings (header + only 6 of the 13 declared string bytes).
    let w = &body_only_wire()[..LOG_HEADER_LEN + 6];
    assert_eq!(
        LogRecord::decode(w),
        Err(LogError::Body(LogReject::BodyLengthMismatch))
    );
}

#[test]
fn trailing_body_bytes_rejected_at_both_layers() {
    // Envelope layer: declared payload shorter than actual input.
    let mut w = LOG_WIRE.to_vec();
    w.push(0xff);
    assert_eq!(
        Frame::parse(&w, &Policy::default()),
        Err(Error::Malformed(Reject::TrailingBytes))
    );
    // Body layer: bare decode with extra bytes past the declared strings.
    let mut w = body_only_wire().to_vec();
    w.push(0xff);
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::BodyLengthMismatch))
    );
}

#[test]
fn zero_build_id_rejected() {
    let mut w = body_only_wire();
    for b in &mut w[0..16] {
        *b = 0;
    }
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::ZeroBuildId))
    );
}

#[test]
fn zero_sequence_rejected() {
    let mut w = body_only_wire();
    for b in &mut w[16..24] {
        *b = 0;
    }
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::ZeroSequence))
    );
}

#[test]
fn monotonic_zero_is_valid() {
    let wire = body_only_wire();
    let mut w = wire;
    for b in &mut w[24..32] {
        *b = 0;
    }
    let r = LogRecord::decode(&w).unwrap();
    assert_eq!(r.monotonic_us(), 0);
}

#[test]
fn invalid_severity_rejected() {
    for sev in [6u8, 7, 100, 255] {
        let mut w = body_only_wire();
        w[32] = sev;
        assert_eq!(
            LogRecord::decode(&w),
            Err(LogError::Body(LogReject::InvalidSeverity))
        );
    }
}

#[test]
fn subsystem_zero_length_and_overlong_rejected() {
    let mut w = body_only_wire();
    w[33] = 0;
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::SubsystemLenOutOfRange))
    );
    w[33] = 65; // > 64
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::SubsystemLenOutOfRange))
    );
}

#[test]
fn message_len_over_cap_rejected_without_overflow() {
    // Declared message 4097 > 4096: compared as u32 BEFORE any conversion.
    let mut w = body_only_wire();
    w[34..38].copy_from_slice(&4097u32.to_le_bytes());
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::MessageLenOverCap))
    );
    // A huge declared length must not wrap or pass via as-cast on any target.
    let mut w = body_only_wire();
    w[34..38].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::MessageLenOverCap))
    );
}

#[test]
fn subsystem_invalid_utf8_rejected() {
    let mut w = body_only_wire();
    w[38] = 0xff; // invalid UTF-8 lead byte in "klog"
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::SubsystemNotUtf8))
    );
}

#[test]
fn subsystem_embedded_nul_rejected() {
    let mut w = body_only_wire();
    w[39] = 0x00; // "k\0og" — valid UTF-8 but contains NUL
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::SubsystemContainsNul))
    );
}

#[test]
fn message_invalid_utf8_rejected_but_nul_and_newline_legal() {
    let mut w = body_only_wire();
    w[42 + 4] = 0xff; // first message byte invalid UTF-8
    assert_eq!(
        LogRecord::decode(&w),
        Err(LogError::Body(LogReject::MessageNotUtf8))
    );
    // The literal wire message already contains \n and \0 and must decode.
    let wire = body_only_wire();
    let r = LogRecord::decode(&wire).unwrap();
    assert!(r.message().contains('\n'));
    assert!(r.message().contains('\0'));
}

#[test]
fn zero_message_length_is_valid() {
    let mut w = body_only_wire().to_vec();
    w[34..38].copy_from_slice(&0u32.to_le_bytes());
    w.truncate(LOG_HEADER_LEN + 4); // drop the 9 message bytes
    let r = LogRecord::decode(&w).unwrap();
    assert_eq!(r.message(), "");
}

#[test]
fn envelope_bounds_enforced_by_frame_before_body() {
    // Declared payload length short of the body's real length.
    let mut w = LOG_WIRE;
    w[32..36].copy_from_slice(&50u32.to_le_bytes()); // short of 51
    assert_eq!(
        Frame::parse(&w, &Policy::default()),
        Err(Error::Malformed(Reject::TrailingBytes))
    );
    // Declared payload length beyond available input.
    let mut w = LOG_WIRE;
    w[32..36].copy_from_slice(&52u32.to_le_bytes());
    assert_eq!(
        Frame::parse(&w, &Policy::default()),
        Err(Error::Malformed(Reject::TruncatedPayload))
    );
}

#[test]
fn envelope_metadata_mismatch_is_visible_not_body_confused() {
    // Same body bytes under a DIFFERENT correlation id and boot id: the
    // bare record is identical, but FramedLog identity differs — so an
    // equal-body check alone could never confirm envelope binding.
    let mut w = LOG_WIRE;
    w[8..16].copy_from_slice(&0xdead_beef_cafe_f00du64.to_le_bytes());
    for i in 0..16 {
        w[16 + i] = 0xf0;
    }
    let f2 = Frame::parse(&w, &Policy::default()).unwrap();
    let fl2 = LogRecord::from_frame(&f2).unwrap();

    let f1 = Frame::parse(&LOG_WIRE, &Policy::default()).unwrap();
    let fl1 = LogRecord::from_frame(&f1).unwrap();
    assert_eq!(*fl1.record(), *fl2.record()); // body equal...
    assert_ne!(fl1.correlation_id(), fl2.correlation_id()); // ...ids are not
    assert_ne!(fl1.boot_id(), fl2.boot_id());
    assert_ne!(fl1, fl2);
}

#[test]
fn encode_rejections_preserve_output_and_success_writes_exact_bytes() {
    let wire = body_only_wire();
    let r = LogRecord::decode(&wire).unwrap();

    // Too-small output: nothing written.
    let mut out = [0xa5; 50];
    assert_eq!(
        r.encode_body(&mut out),
        Err(LogError::Envelope(Error::OutputTooSmall {
            required: 51,
            capacity: 50
        }))
    );
    assert!(out.iter().all(|&b| b == 0xa5));

    // Exact-capacity success reproduces the literal wire body byte-for-byte.
    let mut out = [0xa5; 52];
    assert_eq!(r.encode_body(&mut out), Ok(51));
    assert_eq!(&out[..51], &body_only_wire()[..]);
    assert_eq!(out[51], 0xa5); // tail untouched

    // Envelope encode: rejection (zero correlation id) preserves output.
    let mut env = [0x5c; HEADER_LEN + 51];
    assert_eq!(
        r.encode_envelope(0, &[1; 16], &mut env, &Policy::default()),
        Err(LogError::Envelope(Error::Malformed(
            Reject::ZeroCorrelationId
        )))
    );
    assert!(env.iter().all(|&b| b == 0x5c));

    // Envelope encode: too-small output preserves output.
    let mut env = [0x5c; HEADER_LEN + 50];
    assert_eq!(
        r.encode_envelope(1, &[1; 16], &mut env, &Policy::default()),
        Err(LogError::Envelope(Error::OutputTooSmall {
            required: HEADER_LEN + 51,
            capacity: HEADER_LEN + 50
        }))
    );
    assert!(env.iter().all(|&b| b == 0x5c));

    // Envelope encode success reproduces the literal wire envelope.
    let mut env = [0x5c; HEADER_LEN + 52];
    let boot = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    assert_eq!(
        r.encode_envelope(0x0102_0304_0506_0708, &boot, &mut env, &Policy::default()),
        Ok(LOG_WIRE.len())
    );
    assert_eq!(&env[..LOG_WIRE.len()], &LOG_WIRE[..]);
    assert_eq!(env[LOG_WIRE.len()], 0x5c);
}

#[test]
fn decoder_built_record_reencodes_byte_identically() {
    let wire = body_only_wire();
    let r = LogRecord::decode(&wire).unwrap();
    assert!(MAX_MESSAGE_BYTES <= 4096);
    assert_eq!(MAX_LOG_BODY, LOG_HEADER_LEN + 64 + 4096);
    let mut out = [0u8; MAX_LOG_BODY];
    let n = r.encode_body(&mut out).unwrap();
    assert_eq!(&out[..n], &wire[..]);
}

#[test]
fn max_sized_body_roundtrips() {
    // 64-byte subsystem, 4096-byte message with newlines and NULs.
    let mut w = vec![0u8; LOG_HEADER_LEN + 64 + 4096];
    for i in 0..16 {
        w[i] = 0xaa;
    }
    w[16..24].copy_from_slice(&1u64.to_le_bytes());
    w[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
    w[32] = 5; // Fatal
    w[33] = 64;
    w[34..38].copy_from_slice(&4096u32.to_le_bytes());
    for i in 0..64 {
        w[LOG_HEADER_LEN + i] = b'a' + (i as u8 % 26);
    }
    let msg_start = LOG_HEADER_LEN + 64;
    for i in 0..4096 {
        w[msg_start + i] = match i % 3 {
            0 => b'\n',
            1 => 0x00,
            _ => b'x',
        };
    }
    let r = LogRecord::decode(&w).unwrap();
    assert_eq!(r.severity(), Severity::Fatal);
    assert_eq!(r.monotonic_us(), u64::MAX);
    assert_eq!(r.message().len(), 4096);
    assert_eq!(r.message().as_bytes(), &w[msg_start..]);
    let mut out = [0u8; MAX_LOG_BODY];
    assert_eq!(r.encode_body(&mut out), Ok(w.len()));
    assert_eq!(&out[..w.len()], &w[..]);
}
