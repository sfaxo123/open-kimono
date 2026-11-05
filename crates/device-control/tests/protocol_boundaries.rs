use device_control::{Error, Frame, Op, Policy, Reject, HEADER_LEN, MAX_PAYLOAD};

// Independent literal bytes: no encoder, integer conversion helper or header builder.
const WIRE: [u8; 39] = [
    0x44, 0x43, 0x54, 0x4c, 0x01, 0x00, 0x03, 0x00, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 0x03, 0x00, 0x00, 0x00, 0x61, 0x62,
    0x63,
];

#[test]
fn independent_literal_pins_offsets_and_endianness() {
    let f = Frame::parse(&WIRE, &Policy::default()).unwrap();
    assert_eq!(f.op(), Op::Command);
    assert_eq!(f.correlation_id(), 0x0102_0304_0506_0708);
    assert_eq!(
        f.boot_id(),
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
    assert_eq!(f.payload(), b"abc");
    let mut output = [0xee; 40];
    assert_eq!(
        Frame::encode(
            f.op(),
            f.correlation_id(),
            f.boot_id(),
            f.payload(),
            &mut output,
            &Policy::default()
        ),
        Ok(39)
    );
    assert_eq!(output[..39], WIRE);
    assert_eq!(output[39], 0xee);
}

#[test]
fn exact_hard_cap_is_accepted_and_output_tail_preserved() {
    let payload = vec![0x5a; MAX_PAYLOAD as usize];
    let mut output = vec![0xee; HEADER_LEN + payload.len() + 1];
    let boot = [1; 16];
    let n = Frame::encode(
        Op::Screenshot,
        9,
        &boot,
        &payload,
        &mut output,
        &Policy::default(),
    )
    .unwrap();
    assert_eq!(output[n], 0xee);
    let f = Frame::parse(&output[..n], &Policy::default()).unwrap();
    assert_eq!(f.payload(), payload);
    assert_eq!(f.payload().as_ptr(), output[HEADER_LEN..].as_ptr());
}

#[test]
fn each_rejected_encode_preserves_output_independently() {
    for (id, boot, policy, capacity, want) in [
        (
            0,
            [1; 16],
            Policy::default(),
            40,
            Error::Malformed(Reject::ZeroCorrelationId),
        ),
        (
            1,
            [0; 16],
            Policy::default(),
            40,
            Error::Malformed(Reject::ZeroBootId),
        ),
        (
            1,
            [1; 16],
            Policy { max_payload: 2 },
            40,
            Error::Malformed(Reject::DeclaredLengthOverPolicyCap),
        ),
        (
            1,
            [1; 16],
            Policy {
                max_payload: MAX_PAYLOAD + 1,
            },
            40,
            Error::InvalidPolicy,
        ),
        (
            1,
            [1; 16],
            Policy::default(),
            38,
            Error::OutputTooSmall {
                required: 39,
                capacity: 38,
            },
        ),
    ] {
        let mut output = [0xee; 40];
        assert_eq!(
            Frame::encode(
                Op::Input,
                id,
                &boot,
                b"abc",
                &mut output[..capacity],
                &policy
            ),
            Err(want)
        );
        assert_eq!(output, [0xee; 40]);
    }
}

#[test]
fn repeated_small_envelopes_keep_borrow_identity() {
    let mut output = [0; 39];
    let policy = Policy::default();
    for _ in 0..10_000 {
        let f = Frame::parse(std::hint::black_box(&WIRE), &policy).unwrap();
        assert_eq!(f.payload().as_ptr(), WIRE[HEADER_LEN..].as_ptr());
        assert_eq!(
            Frame::encode(
                f.op(),
                f.correlation_id(),
                f.boot_id(),
                f.payload(),
                &mut output,
                &policy
            ),
            Ok(39)
        );
        assert_eq!(output, WIRE);
    }
}
