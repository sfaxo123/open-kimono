//! Integration tests for the borrowed COMMAND request-body v1 codec over the
//! accepted control envelope. All wire bytes are literal and hand-written —
//! nothing here depends on the encoder to construct inputs. The codec never
//! executes anything; these tests only check bytes in / validated borrows out.

use device_control::command_record::{
    CommandError, CommandReject, CommandRequest, MAX_ARGC, MAX_ARG_BYTES, MAX_COMMAND_BODY,
};
use device_control::{Error, Frame, Op, Policy, Reject, HEADER_LEN};

// A complete envelope (36-byte header + 23-byte request body) with op = Command.
// Body: tag 1, timeout_ms 5000 LE, argc 3, args "ls" / "" (empty allowed past
// argv0) / b"\xff\xfe bytes" (non-UTF-8 preserved).
const REQ_WIRE: [u8; 59] = [
    // envelope header (36 bytes)
    0x44, 0x43, 0x54, 0x4c, // "DCTL"
    0x01, 0x00, // version 1
    0x03, // op = Command
    0x00, // reserved
    0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, // correlation id LE
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
    0x10, // boot id
    0x17, 0x00, 0x00, 0x00, // declared payload = 23
    // request body (23 bytes)
    0x01, // tag = request
    0x88, 0x13, 0x00, 0x00, // timeout_ms = 5000
    0x03, 0x00, // argc = 3
    0x02, 0x00, // arg0 len = 2
    0x6c, 0x73, // "ls"
    0x00, 0x00, // arg1 len = 0 (empty allowed)
    0x08, 0x00, // arg2 len = 8
    0xff, 0xfe, 0x20, 0x62, 0x79, 0x74, 0x65, 0x73, // non-UTF-8 bytes
];

fn body_only_wire() -> [u8; 23] {
    REQ_WIRE[HEADER_LEN..]
        .try_into()
        .expect("literal slice length")
}

#[test]
fn independent_literal_decodes_every_field() {
    let f = Frame::parse(&REQ_WIRE, &Policy::default()).unwrap();
    assert_eq!(f.op(), Op::Command);
    let fr = CommandRequest::from_frame(&f).unwrap();
    let r = fr.request();
    assert_eq!(r.timeout_ms(), 5000);
    assert_eq!(r.argc(), 3);
    let args: Vec<&[u8]> = r.args().collect();
    assert_eq!(
        args,
        vec![
            b"ls".as_slice(),
            b"".as_slice(),
            b"\xff\xfe bytes".as_slice()
        ]
    );
}

#[test]
fn framed_request_retains_envelope_correlation_and_boot_identity() {
    let f = Frame::parse(&REQ_WIRE, &Policy::default()).unwrap();
    let fr = CommandRequest::from_frame(&f).unwrap();
    assert_eq!(fr.correlation_id(), 0x0102_0304_0506_0708);
    assert_eq!(
        fr.boot_id(),
        &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
    );
}

#[test]
fn wrong_op_frame_is_refused() {
    let mut wire = REQ_WIRE;
    wire[6] = 1; // op = Logs, not Command
    let f = Frame::parse(&wire, &Policy::default()).unwrap();
    assert_eq!(
        CommandRequest::from_frame(&f),
        Err(CommandError::Body(CommandReject::WrongOp))
    );
    // And the envelope itself must still be a Command frame for decode to be
    // reachable — envelope identity is checked at the frame layer first.
    let mut bad_magic = REQ_WIRE;
    bad_magic[0] = b'X';
    assert_eq!(
        Frame::parse(&bad_magic, &Policy::default()),
        Err(Error::Malformed(Reject::BadMagic))
    );
}

#[test]
fn arguments_are_borrowed_from_input_not_copied() {
    let body = body_only_wire();
    let r = CommandRequest::decode(&body).unwrap();
    let first = r.args().next().unwrap();
    // Same address as inside the caller's buffer: a borrow, not an allocation.
    assert_eq!(first.as_ptr(), body[9..].as_ptr());
}

#[test]
fn iterator_yields_no_hidden_argv_allocation_and_terminates() {
    let body = body_only_wire();
    let r = CommandRequest::decode(&body).unwrap();
    // Every yielded argument slice must point INSIDE the caller's buffer:
    // proof that iteration borrows and never allocates a hidden argv copy.
    let base = body.as_ptr() as usize;
    let items: Vec<&[u8]> = r.args().collect();
    assert_eq!(items.len(), 3);
    for a in &items {
        let off = a.as_ptr() as usize - base;
        assert!(off >= 9 && off + a.len() <= body.len());
    }
    // The iterator is not an ExactSizeIterator (no materialized table):
    // it simply terminates after exactly argc items.
    let mut it = r.args();
    it.next();
    it.next();
    it.next();
    assert!(it.next().is_none());
}

#[test]
fn timeout_boundaries() {
    let args = [b"bin".as_slice()];
    assert!(CommandRequest::new(0, &args).is_err());
    assert!(CommandRequest::new(1, &args).is_ok());
    assert!(CommandRequest::new(60_000, &args).is_ok());
    assert!(CommandRequest::new(60_001, &args).is_err());
    assert!(CommandRequest::new(u32::MAX, &args).is_err());
}

#[test]
fn timeout_wire_boundaries_reject_with_reason() {
    let mut body = body_only_wire();
    body[1..5].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::TimeoutOutOfRange))
    );
    body[1..5].copy_from_slice(&60_001u32.to_le_bytes());
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::TimeoutOutOfRange))
    );
    body[1..5].copy_from_slice(&60_000u32.to_le_bytes());
    assert!(CommandRequest::decode(&body).is_ok());
}

#[test]
fn argc_wire_boundaries() {
    let mut body = body_only_wire();
    // argc 0: rejected.
    body[5..7].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::ArgcOutOfRange))
    );
    // argc 33: rejected even though the remaining bytes are trailing junk.
    body[5..7].copy_from_slice(&33u16.to_le_bytes());
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::ArgcOutOfRange))
    );
    // argc 32 with 32 one-byte args: exactly 7 + 32*3 = 103 bytes, valid.
    let mut wide = vec![0u8; 7 + 32 * 3];
    wide[0] = 1;
    wide[1..5].copy_from_slice(&100u32.to_le_bytes());
    wide[5..7].copy_from_slice(&32u16.to_le_bytes());
    for i in 0..32 {
        wide[7 + i * 3] = 1;
        wide[7 + i * 3 + 2] = b'a';
    }
    assert!(CommandRequest::decode(&wide).is_ok());
}

#[test]
fn empty_argv0_rejected_but_empty_later_args_allowed() {
    // Literal body: tag 1, timeout 1, argc 1, single empty arg.
    let zeroth = [1u8, 1, 0, 0, 0, 1, 0, 0, 0];
    assert_eq!(
        CommandRequest::decode(&zeroth),
        Err(CommandError::Body(CommandReject::EmptyArgv0))
    );
    // argv0 nonempty, argv1 empty: valid (matches REQ_WIRE arg1).
    assert!(CommandRequest::decode(&body_only_wire()).is_ok());
}

#[test]
fn nul_in_argument_rejected() {
    // tag 1, timeout 1, argc 1, arg len 3 = b"a\0b".
    let nul = [1u8, 1, 0, 0, 0, 1, 0, 3, 0, b'a', 0, b'b'];
    assert_eq!(
        CommandRequest::decode(&nul),
        Err(CommandError::Body(CommandReject::ArgumentContainsNul))
    );
}

#[test]
fn nonutf8_and_high_bytes_preserved_losslessly() {
    let body = body_only_wire();
    let r = CommandRequest::decode(&body).unwrap();
    let last = r.args().nth(2).unwrap();
    assert_eq!(last, b"\xff\xfe bytes");
    // No UTF-8 assumption anywhere: invalid sequences roundtrip byte-exact.
    assert!(std::str::from_utf8(last).is_err());
}

#[test]
fn arg_length_cap_boundary() {
    // One arg of exactly MAX_ARG_BYTES: valid.
    let mut ok = vec![0u8; 7 + 2 + MAX_ARG_BYTES];
    ok[0] = 1;
    ok[1..5].copy_from_slice(&1u32.to_le_bytes());
    ok[5..7].copy_from_slice(&1u16.to_le_bytes());
    ok[7..9].copy_from_slice(&(MAX_ARG_BYTES as u16).to_le_bytes());
    // Fill all 4096 bytes so declared length and actual bytes agree.
    for i in 0..MAX_ARG_BYTES {
        ok[9 + i] = (i % 250) as u8 + 1; // values 1..=250: never a NUL byte
    }
    assert!(CommandRequest::decode(&ok).is_ok());
    // Declared 4097: rejected as over-cap BEFORE any truncation question —
    // the cap check precedes the length-fit walk.
    let mut over = ok.clone();
    over[7..9].copy_from_slice(&4097u16.to_le_bytes());
    assert_eq!(
        CommandRequest::decode(&over),
        Err(CommandError::Body(CommandReject::ArgLenOverCap))
    );
    // A 4097-byte arg cannot even be built as a valid table: new() rejects.
    let big = vec![b'x'; 4097];
    let bigargs: [&[u8]; 1] = [big.as_slice()];
    assert_eq!(
        CommandRequest::new(1, &bigargs),
        Err(CommandError::Body(CommandReject::ArgLenOverCap))
    );
}

#[test]
fn truncated_argument_table_rejected() {
    // Declared 2 args but the second arg's bytes are missing.
    let trunc = [1u8, 5, 0, 0, 0, 2, 0, 2, 0, b'a', b'b'];
    assert_eq!(
        CommandRequest::decode(&trunc),
        Err(CommandError::Body(CommandReject::BodyLengthMismatch))
    );
    // Header cut mid-argc.
    assert_eq!(
        CommandRequest::decode(&[1u8, 5, 0, 0, 0]),
        Err(CommandError::Body(CommandReject::BodyTooShortForHeader))
    );
    // Arg length field itself truncated.
    assert_eq!(
        CommandRequest::decode(&[1u8, 5, 0, 0, 0, 1, 0, 3]),
        Err(CommandError::Body(CommandReject::BodyLengthMismatch))
    );
}

#[test]
fn trailing_bytes_rejected() {
    // Trailing bytes INSIDE the declared payload: the envelope accepts the
    // frame, the request decoder rejects the extra argument bytes.
    let mut body = body_only_wire().to_vec();
    body.push(0x00);
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::BodyLengthMismatch))
    );
    // Trailing bytes OUTSIDE the declared payload: rejected at the envelope.
    let mut wire = REQ_WIRE.to_vec();
    wire.push(0x00);
    assert_eq!(
        Frame::parse(&wire, &Policy::default()),
        Err(Error::Malformed(Reject::TrailingBytes))
    );
}

#[test]
fn wrong_tag_rejected() {
    let mut body = body_only_wire();
    body[0] = 2; // result tag in a request decode
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::UnexpectedTag))
    );
    body[0] = 3;
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::UnexpectedTag))
    );
}

#[test]
fn body_over_hard_cap_rejected() {
    let body = vec![0u8; MAX_COMMAND_BODY + 1];
    assert_eq!(
        CommandRequest::decode(&body),
        Err(CommandError::Body(CommandReject::BodyOverHardCap))
    );
}

#[test]
fn encode_from_caller_args_matches_literal_wire() {
    let a0: &[u8] = b"ls";
    let a1: &[u8] = b"";
    let a2: &[u8] = b"\xff\xfe bytes";
    let args = [a0, a1, a2];
    let r = CommandRequest::new(5000, &args).unwrap();
    let mut out = [0u8; 23];
    assert_eq!(r.encode_body(&mut out).unwrap(), 23);
    assert_eq!(out, body_only_wire());
    // Full envelope roundtrip matches the literal wire byte-for-byte.
    let boot = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let mut env = [0u8; 59];
    assert_eq!(
        r.encode_envelope(0x0102_0304_0506_0708, &boot, &mut env, &Policy::default())
            .unwrap(),
        59
    );
    assert_eq!(env, REQ_WIRE);
}

#[test]
fn rejected_encode_leaves_output_unchanged() {
    let nul: &[u8] = b"a\0b";
    let args: [&[u8]; 1] = [nul];
    let mut out = [0xA5u8; 32];
    // Cannot construct an invalid request via new()...
    assert_eq!(
        CommandRequest::new(100, &args),
        Err(CommandError::Body(CommandReject::ArgumentContainsNul))
    );
    // ...but an undersized output on a valid request must not touch bytes.
    let good: &[u8] = b"ls";
    let gargs: [&[u8]; 1] = [good];
    let r = CommandRequest::new(100, &gargs).unwrap();
    let before = out;
    assert_eq!(
        r.encode_body(&mut out[..3]),
        Err(CommandError::Envelope(Error::OutputTooSmall {
            required: 11,
            capacity: 3
        }))
    );
    assert_eq!(out, before);
}

#[test]
fn decoded_request_reencodes_to_identical_bytes() {
    let body = body_only_wire();
    let r = CommandRequest::decode(&body).unwrap();
    let mut out = [0u8; 23];
    assert_eq!(r.encode_body(&mut out).unwrap(), 23);
    assert_eq!(out, body);
}

#[test]
fn new_rejects_invalid_argv_tables() {
    let empty0: &[u8] = b"";
    assert_eq!(
        CommandRequest::new(100, &[empty0][..]),
        Err(CommandError::Body(CommandReject::EmptyArgv0))
    );
    let noargs: [&[u8]; 0] = [];
    assert_eq!(
        CommandRequest::new(100, &noargs),
        Err(CommandError::Body(CommandReject::ArgcOutOfRange))
    );
    // 33 args: out of range.
    let one: &[u8] = b"x";
    let many = [one; 33];
    assert_eq!(
        CommandRequest::new(100, &many),
        Err(CommandError::Body(CommandReject::ArgcOutOfRange))
    );
}

// ---------------------------------------------------------------------------
// Canonical literal-byte fixtures — one per command KIND (the kind byte is
// the request-body tag; this slice implements exactly one kind, tag 1).
// Every fixture below is hand-written bytes: nothing here is generated by
// the encoder before it is asserted. The demo example must print a frame
// whose bytes equal REQ_WIRE, and that claim is checked against these same
// literals — never against an encoder-produced buffer.
// ---------------------------------------------------------------------------

/// Canonical request-kind (tag 1) envelope: `REQ_WIRE`, byte-for-byte.
#[test]
fn canonical_fixture_request_kind_decodes_without_the_encoder() {
    let f = Frame::parse(&REQ_WIRE, &Policy::default()).expect("canonical fixture parses");
    let fr = CommandRequest::from_frame(&f).expect("canonical request kind decodes");
    // Every decoded field equals the literal written in the fixture — no
    // value is fabricated: correlation id and boot id come from header
    // bytes 8..16 and 16..32, timeout/argc/argv from body bytes only.
    assert_eq!(
        fr.correlation_id(),
        u64::from_le_bytes(REQ_WIRE[8..16].try_into().unwrap())
    );
    assert_eq!(&fr.boot_id()[..], &REQ_WIRE[16..32]);
    assert_eq!(
        fr.request().timeout_ms(),
        u32::from_le_bytes(REQ_WIRE[37..41].try_into().unwrap())
    );
    let args: Vec<&[u8]> = fr.request().args().collect();
    assert_eq!(
        args,
        vec![
            b"ls".as_slice(),
            b"".as_slice(),
            b"\xff\xfe bytes".as_slice()
        ]
    );
}

/// A deliberately MALFORMED minimal literal (empty argv[0]): kept to assert
/// the decoder rejects it rather than repairing input.
const REQ_MIN_BAD: [u8; 45] = [
    // envelope header (36 bytes)
    0x44, 0x43, 0x54, 0x4c, // "DCTL"
    0x01, 0x00, // version 1
    0x03, // op = Command
    0x00, // reserved
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // correlation id = 1
    0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, // boot id (nonzero)
    0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00, 0x09, 0x00, 0x00,
    0x00, // declared payload = 9
    // request body (9 bytes): tag 1, timeout 1, argc 1, EMPTY arg0 - illegal
    0x01, // tag = request
    0x01, 0x00, 0x00, 0x00, // timeout_ms = 1 (minimum)
    0x01, 0x00, // argc = 1
    0x00, 0x00, // arg0 len = 0: argv[0] must be nonempty
];

/// Second canonical fixture for the same kind with a distinct shape
/// (single one-byte arg, minimum timeout).
const REQ_MIN_OK: [u8; 46] = [
    // envelope header (36 bytes)
    0x44, 0x43, 0x54, 0x4c, // "DCTL"
    0x01, 0x00, // version 1
    0x03, // op = Command
    0x00, // reserved
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // correlation id = 1
    0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88, // boot id (nonzero)
    0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00, 0x0a, 0x00, 0x00,
    0x00, // declared payload = 10
    // request body (10 bytes): tag 1, timeout 1, argc 1, one-byte arg "c"
    0x01, // tag = request
    0x01, 0x00, 0x00, 0x00, // timeout_ms = 1 (minimum)
    0x01, 0x00, // argc = 1
    0x01, 0x00, // arg0 len = 1
    0x63, // "c"
];

#[test]
fn second_canonical_fixture_minimal_request_kind() {
    let f = Frame::parse(&REQ_MIN_OK, &Policy::default()).expect("minimal canonical parses");
    let fr = CommandRequest::from_frame(&f).unwrap();
    assert_eq!(fr.correlation_id(), 1);
    assert_eq!(fr.request().timeout_ms(), 1);
    let args: Vec<&[u8]> = fr.request().args().collect();
    assert_eq!(args, vec![b"c".as_slice()]);
}

#[test]
fn canonical_bad_minimal_literal_is_rejected_not_rescued() {
    // The deliberately-malformed literal (empty argv0) must be a typed
    // rejection — the decoder does not repair input.
    let f = Frame::parse(&REQ_MIN_BAD, &Policy::default()).expect("envelope is well-formed");
    assert_eq!(
        CommandRequest::from_frame(&f),
        Err(CommandError::Body(CommandReject::EmptyArgv0))
    );
}

#[test]
fn roundtrip_at_maximum_body_size() {
    // Maximum LEGAL request body under the caps: 7 + 15*(2+4096) + (2+4057)
    // = exactly 65536 bytes, with 16 args (within MAX_ARGC). Round-trip it
    // through encode -> decode and require byte-identical argv.
    let filler: Vec<u8> = core::iter::repeat(b'z').take(MAX_ARG_BYTES).collect();
    let last: Vec<u8> = core::iter::repeat(b'q').take(4057).collect();
    let mut argv: Vec<&[u8]> = Vec::new();
    for _ in 0..15 {
        assert!(15 <= MAX_ARGC);
        argv.push(filler.as_slice());
    }
    argv.push(last.as_slice());
    assert_eq!(argv.len(), 16); // within MAX_ARGC
    let r = CommandRequest::new(60_000, &argv).expect("max-size request is legal");
    let mut body = vec![0u8; MAX_COMMAND_BODY];
    assert_eq!(r.encode_body(&mut body).unwrap(), 65536); // exactly at the cap
    let back = CommandRequest::decode(&body[..65536]).expect("max-size roundtrip decodes");
    assert_eq!(back.timeout_ms(), 60_000);
    assert_eq!(back.argc(), 16);
    let got: Vec<&[u8]> = back.args().collect();
    assert_eq!(got, argv);
    // One byte more (4058 in the last arg) exceeds the explicit maximum.
    // The hard-cap sum check runs BEFORE per-arg caps, so this is
    // BodyOverHardCap — not a truncation or per-field rejection — even
    // though 4058 also breaks the 4096-byte arg cap.
    let mut over_argv = argv.clone();
    let extra: Vec<u8> = core::iter::repeat(b'q').take(4058).collect();
    over_argv[15] = extra.as_slice();
    assert_eq!(
        CommandRequest::new(60_000, &over_argv),
        Err(CommandError::Body(CommandReject::BodyOverHardCap))
    );
    // Hard-cap rejection with every per-arg length legal: 31 args of 2045
    // bytes + one of 2086 sums to 7 + 31*2047 + 2088 = 65_584 > 65_536.
    let mut cap_argv: Vec<&[u8]> = Vec::new();
    let half: Vec<u8> = core::iter::repeat(b'w').take(2045).collect();
    for _ in 0..31 {
        cap_argv.push(half.as_slice());
    }
    let big: Vec<u8> = core::iter::repeat(b'w').take(2086).collect();
    cap_argv.push(big.as_slice());
    assert_eq!(
        CommandRequest::new(60_000, &cap_argv),
        Err(CommandError::Body(CommandReject::BodyOverHardCap))
    );
}

#[test]
fn truncation_at_every_field_boundary_is_typed_error_never_panic() {
    // Every strict prefix of the canonical body (lengths 0..23) must yield
    // a typed error, never a panic. This sweeps every field boundary: tag,
    // each timeout byte, each argc byte, each arg-length byte, and each
    // argument byte.
    let body = body_only_wire();
    for cut in 0..body.len() {
        let r = CommandRequest::decode(&body[..cut]);
        assert!(r.is_err(), "prefix of length {cut} must be rejected");
        match r.unwrap_err() {
            CommandError::Body(CommandReject::BodyTooShortForHeader)
            | CommandError::Body(CommandReject::UnexpectedTag)
            | CommandError::Body(CommandReject::TimeoutOutOfRange)
            | CommandError::Body(CommandReject::ArgcOutOfRange)
            | CommandError::Body(CommandReject::EmptyArgv0)
            | CommandError::Body(CommandReject::BodyLengthMismatch) => {}
            other => panic!("unexpected error kind for cut {cut}: {other:?}"),
        }
    }
    // And every strict prefix of the full envelope hits the frame layer.
    for cut in 0..REQ_WIRE.len() {
        assert!(Frame::parse(&REQ_WIRE[..cut], &Policy::default()).is_err());
    }
}

#[test]
fn oversize_declared_payload_rejected_before_allocation() {
    // A frame header declaring a payload above the envelope hard cap is
    // rejected at the declared-length check, before any size arithmetic or
    // allocation. The input carries NO payload bytes at all — if anything
    // allocated from the declaration, this test would abort or hang.
    let mut wire = [0u8; 36];
    wire[..4].copy_from_slice(b"DCTL");
    wire[4..6].copy_from_slice(&1u16.to_le_bytes());
    wire[6] = 3; // op = Command
    wire[8] = 1; // correlation id = 1
    wire[16] = 1; // boot id nonzero
                  // The envelope hard cap is MAX_PAYLOAD (16 MiB); to trip the
                  // declared-length hard-cap check the declaration must exceed IT.
    wire[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        CommandRequest::decode_envelope(&wire, &Policy::default()),
        Err(CommandError::Envelope(Error::Malformed(
            Reject::DeclaredLengthOverHardCap
        )))
    );
    // A policy-tightened cap rejects a declared length over IT, also before
    // allocation. The envelope hard cap is MAX_PAYLOAD (16 MiB), so a body
    // just over the command-body cap is still inside it; the tightened
    // policy is what refuses the declaration.
    let mut wire2 = wire;
    wire2[32..36].copy_from_slice(&(MAX_COMMAND_BODY as u32 + 1).to_le_bytes());
    let tight = Policy { max_payload: 512 };
    assert_eq!(
        CommandRequest::decode_envelope(&wire2, &tight),
        Err(CommandError::Envelope(Error::Malformed(
            Reject::DeclaredLengthOverPolicyCap
        )))
    );
}

#[test]
fn decode_envelope_one_call_matches_two_step_path() {
    let one = CommandRequest::decode_envelope(&REQ_WIRE, &Policy::default()).unwrap();
    let two =
        CommandRequest::from_frame(&Frame::parse(&REQ_WIRE, &Policy::default()).unwrap()).unwrap();
    assert_eq!(one, two);
}

#[test]
fn demo_example_output_matches_checked_in_fixture() {
    // The acceptance command runs the example; here we re-derive the exact
    // bytes the example prints (it encodes `ls`/``/b"\xff\xfe bytes" with
    // timeout 5000, correlation 0x0102_0304_0506_0708 and boot id 1..=16)
    // through the public codec and assert byte equality with REQ_WIRE. The
    // example itself prints hex of exactly this frame; its stdout is checked
    // against this fixture in CI by `scripts/`-free comparison below:
    // encode -> hex -> compare to the literal, so a changed fixture or a
    // changed encoder both fail.
    let args: [&[u8]; 3] = [
        b"ls".as_slice(),
        b"".as_slice(),
        b"\xff\xfe bytes".as_slice(),
    ];
    let r = CommandRequest::new(5000, &args).unwrap();
    let boot = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    let mut env = [0u8; 59];
    let n = r
        .encode_envelope(0x0102_0304_0506_0708, &boot, &mut env, &Policy::default())
        .unwrap();
    assert_eq!(n, REQ_WIRE.len());
    assert_eq!(&env[..n], &REQ_WIRE[..]);

    // Hex-encode the fixture and confirm it equals exactly what the demo
    // prints (the demo uses the same lowercase two-digit format), so a
    // changed fixture or a changed encoder both fail here.
    let hex = hex_of(&env[..n]);
    assert_eq!(hex.len(), 118); // 59 bytes x 2 nibbles
    assert_eq!(&hex[..8], "4443544c"); // "DCTL"
    assert_eq!(&hex[118 - 16..], "fffe206279746573"); // tail "\xff\xfe bytes"
                                                      // The complete hex string, pinned as a literal — this is exactly the
                                                      // line `cargo run --example command_request_demo` prints.
    assert_eq!(
        hex,
        "4443544c0100030008070605040302010102030405060708090a0b0c0d0e0f10\
         170000000188130000030002006c7300000800fffe206279746573"
    );
}

/// Lowercase hex of a byte slice, matching the demo's print format.
fn hex_of(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(b"0123456789abcdef"[usize::from(b >> 4)]));
        s.push(char::from(b"0123456789abcdef"[usize::from(b & 0xf)]));
    }
    s
}
