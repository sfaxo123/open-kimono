//! Literal wire vectors for control-envelope v1. These bytes are hand-written
//! fixtures, not encoder round-trips: they pin the exact wire format.

use device_control::{Error, Frame, Op, Policy, Reject, HEADER_LEN, MAX_PAYLOAD};

const BOOT: [u8; 16] = [
    0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x01,
];

/// Hand-built header: magic, version, op, reserved, LE id, boot, LE payload len.
fn header(op: u8, id: u64, boot: &[u8; 16], len: u32) -> Vec<u8> {
    let mut h = vec![b'D', b'C', b'T', b'L', 1, 0, op, 0];
    h.extend_from_slice(&id.to_le_bytes());
    h.extend_from_slice(boot);
    h.extend_from_slice(&len.to_le_bytes());
    assert_eq!(h.len(), HEADER_LEN);
    h
}

fn ok_policy() -> Policy {
    Policy::default()
}

#[test]
fn literal_valid_wire_empty_payload() {
    let wire = header(2, 0x0102_0304_0506_0708, &BOOT, 0);
    let f = Frame::parse(&wire, &ok_policy()).expect("valid");
    assert_eq!(f.op(), Op::Screenshot);
    assert_eq!(f.correlation_id(), 0x0102_0304_0506_0708);
    assert_eq!(f.boot_id(), &BOOT);
    assert!(f.payload().is_empty());
}

#[test]
fn literal_valid_wire_with_payload_and_all_ops() {
    for (opb, want) in [
        (1u8, Op::Logs),
        (2, Op::Screenshot),
        (3, Op::Command),
        (4, Op::Input),
    ] {
        let mut wire = header(opb, 7, &BOOT, 3);
        wire.extend_from_slice(b"abc");
        let f = Frame::parse(&wire, &ok_policy()).expect("valid");
        assert_eq!(f.op(), want);
        assert_eq!(f.payload(), b"abc");
    }
}

#[test]
fn bad_magic_rejected() {
    let mut wire = header(1, 7, &BOOT, 0);
    wire[3] = b'X';
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::BadMagic))
    );
}

#[test]
fn unsupported_version_rejected() {
    for v in [0u16, 2, 0xFFFF] {
        let mut wire = header(1, 7, &BOOT, 0);
        wire[4..6].copy_from_slice(&v.to_le_bytes());
        assert_eq!(
            Frame::parse(&wire, &ok_policy()),
            Err(Error::Malformed(Reject::UnsupportedVersion))
        );
    }
}

#[test]
fn unsupported_op_rejected() {
    for opb in [0u8, 5, 255] {
        let wire = header(opb, 7, &BOOT, 0);
        assert_eq!(
            Frame::parse(&wire, &ok_policy()),
            Err(Error::Malformed(Reject::UnsupportedOp))
        );
    }
}

#[test]
fn reserved_nonzero_rejected() {
    let mut wire = header(1, 7, &BOOT, 0);
    wire[7] = 1;
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::ReservedNonZero))
    );
}

#[test]
fn zero_correlation_id_rejected() {
    let wire = header(1, 0, &BOOT, 0);
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::ZeroCorrelationId))
    );
}

#[test]
fn zero_boot_id_rejected() {
    let wire = header(1, 7, &[0u8; 16], 0);
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::ZeroBootId))
    );
}

#[test]
fn every_truncated_header_length_rejected() {
    let wire = header(1, 7, &BOOT, 0);
    for len in 0..HEADER_LEN {
        assert_eq!(
            Frame::parse(&wire[..len], &ok_policy()),
            Err(Error::Malformed(Reject::InputTooShortForHeader)),
            "header length {len}"
        );
    }
}

#[test]
fn payload_truncation_rejected() {
    let mut wire = header(1, 7, &BOOT, 10);
    wire.extend_from_slice(b"short");
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::TruncatedPayload))
    );
}

#[test]
fn trailing_bytes_rejected() {
    let mut wire = header(1, 7, &BOOT, 1);
    wire.extend_from_slice(b"aX");
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::TrailingBytes))
    );
}

#[test]
fn declared_max_u32_rejected_under_hard_and_tight_policy() {
    let wire = header(1, 7, &BOOT, u32::MAX);
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::DeclaredLengthOverHardCap))
    );
    let tight = Policy { max_payload: 64 };
    assert_eq!(
        Frame::parse(&wire, &tight),
        Err(Error::Malformed(Reject::DeclaredLengthOverHardCap))
    );
}

#[test]
fn hard_cap_boundary() {
    let mut wire = header(1, 7, &BOOT, MAX_PAYLOAD);
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::TruncatedPayload))
    );
    wire[32..36].copy_from_slice(&(MAX_PAYLOAD + 1).to_le_bytes());
    assert_eq!(
        Frame::parse(&wire, &ok_policy()),
        Err(Error::Malformed(Reject::DeclaredLengthOverHardCap))
    );
}

#[test]
fn tight_policy_rejects_larger_payload() {
    let mut wire = header(1, 7, &BOOT, 100);
    wire.extend_from_slice(&[7u8; 100]);
    let tight = Policy { max_payload: 64 };
    assert_eq!(
        Frame::parse(&wire, &tight),
        Err(Error::Malformed(Reject::DeclaredLengthOverPolicyCap))
    );
    assert!(Frame::parse(&wire, &ok_policy()).is_ok());
}

#[test]
fn loose_policy_is_rejected_not_clamped() {
    let loose = Policy {
        max_payload: MAX_PAYLOAD + 1,
    };
    assert_eq!(
        Frame::parse(&header(1, 7, &BOOT, 0), &loose),
        Err(Error::InvalidPolicy)
    );
    let mut out = *b"SENTINELSENTINELSENTINELSENTINEL";
    assert_eq!(
        Frame::encode(Op::Logs, 7, &BOOT, b"x", &mut out, &loose),
        Err(Error::InvalidPolicy)
    );
    assert_eq!(out, *b"SENTINELSENTINELSENTINELSENTINEL");
    let looser = Policy {
        max_payload: u32::MAX,
    };
    assert_eq!(
        Frame::parse(&header(1, 7, &BOOT, 0), &looser),
        Err(Error::InvalidPolicy)
    );
}

#[test]
fn payload_is_borrowed_not_copied() {
    let mut wire = header(3, 7, &BOOT, 4);
    wire.extend_from_slice(b"data");
    let f = Frame::parse(&wire, &ok_policy()).expect("valid");
    assert!(f.payload().as_ptr() == wire[HEADER_LEN..].as_ptr());
}

#[test]
fn encoder_roundtrip_matches_literal_wire() {
    let mut out = [0u8; 64];
    let n = Frame::encode(
        Op::Command,
        0x0102_0304_0506_0708,
        &BOOT,
        b"hi",
        &mut out,
        &ok_policy(),
    )
    .expect("encode");
    assert_eq!(n, HEADER_LEN + 2);
    let mut want = header(3, 0x0102_0304_0506_0708, &BOOT, 2);
    want.extend_from_slice(b"hi");
    assert_eq!(&out[..n], &want[..]);
}

#[test]
fn rejected_encode_leaves_output_unchanged() {
    let mut out = *b"SENTINELSENTINELSENTINELSENTINEL";
    let before = out;
    let bad_boot = [0u8; 16];
    assert_eq!(
        Frame::encode(Op::Logs, 7, &bad_boot, b"x", &mut out, &ok_policy()),
        Err(Error::Malformed(Reject::ZeroBootId))
    );
    assert_eq!(
        Frame::encode(Op::Logs, 0, &BOOT, b"x", &mut out, &ok_policy()),
        Err(Error::Malformed(Reject::ZeroCorrelationId))
    );
    let tight = Policy { max_payload: 2 };
    assert_eq!(
        Frame::encode(Op::Logs, 7, &BOOT, b"toolong", &mut out, &tight),
        Err(Error::Malformed(Reject::DeclaredLengthOverPolicyCap))
    );
    assert_eq!(
        Frame::encode(Op::Logs, 7, &BOOT, b"toolong", &mut out, &ok_policy()),
        Err(Error::OutputTooSmall {
            required: HEADER_LEN + 7,
            capacity: 32
        })
    );
    assert_eq!(
        out, before,
        "output must be untouched after every rejection"
    );
}

#[test]
fn encoder_rejects_oversized_payload_under_hard_cap() {
    let tight = Policy {
        max_payload: MAX_PAYLOAD,
    };
    let big = vec![0u8; MAX_PAYLOAD as usize + 1];
    let mut out = [0u8; 1];
    assert_eq!(
        Frame::encode(Op::Logs, 7, &BOOT, &big, &mut out, &tight),
        Err(Error::Malformed(Reject::DeclaredLengthOverHardCap))
    );
}

#[test]
fn unsupported_transport_reported_honestly() {
    for name in ["usb", "adb", "tcp", "uart", "carrier-pigeon"] {
        match device_control::transport_report(name) {
            Error::UnsupportedTransport(_) => {}
            other => panic!("expected UnsupportedTransport, got {other:?}"),
        }
    }
}
