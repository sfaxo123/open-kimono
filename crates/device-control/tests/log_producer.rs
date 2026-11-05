use device_control::log_record::{LogError, LogRecord, LogReject, Severity, MAX_LOG_BODY};
use device_control::{Frame, Policy, HEADER_LEN};

#[test]
fn producer_encodes_each_application_severity_without_losing_text() {
    for (byte, severity) in [
        Severity::Trace,
        Severity::Debug,
        Severity::Info,
        Severity::Warning,
        Severity::Error,
        Severity::Fatal,
    ]
    .into_iter()
    .enumerate()
    {
        let record =
            LogRecord::new([2; 16], 1, 0, severity, "kernel", "failure\n\0detail").unwrap();
        let mut output = [0; HEADER_LEN + MAX_LOG_BODY];
        let n = record
            .encode_envelope(7, &[3; 16], &mut output, &Policy::default())
            .unwrap();
        assert_eq!(output[HEADER_LEN + 32], byte as u8);
        let frame = Frame::parse(&output[..n], &Policy::default()).unwrap();
        let received = LogRecord::from_frame(&frame).unwrap();
        assert_eq!(received.record(), &record);
        assert_eq!(received.correlation_id(), 7);
        assert_eq!(received.boot_id(), &[3; 16]);
    }
}

#[test]
fn producer_rejects_invalid_metadata_and_counts_utf8_bytes() {
    let make =
        |build, seq, sub, message| LogRecord::new(build, seq, 0, Severity::Debug, sub, message);
    assert_eq!(
        make([0; 16], 1, "k", ""),
        Err(LogError::Body(LogReject::ZeroBuildId))
    );
    assert_eq!(
        make([1; 16], 0, "k", ""),
        Err(LogError::Body(LogReject::ZeroSequence))
    );
    assert_eq!(
        make([1; 16], 1, "", ""),
        Err(LogError::Body(LogReject::SubsystemLenOutOfRange))
    );
    assert_eq!(
        make([1; 16], 1, "k\0x", ""),
        Err(LogError::Body(LogReject::SubsystemContainsNul))
    );
    let sub = "é".repeat(33);
    assert_eq!(
        make([1; 16], 1, &sub, ""),
        Err(LogError::Body(LogReject::SubsystemLenOutOfRange))
    );
    let msg = "é".repeat(2049);
    assert_eq!(
        make([1; 16], 1, "k", &msg),
        Err(LogError::Body(LogReject::MessageLenOverCap))
    );
}

#[test]
fn smallest_body_and_exact_utf8_limits_are_accepted() {
    let record = LogRecord::new([1; 16], 1, 0, Severity::Trace, "k", "").unwrap();
    let mut output = [0; MAX_LOG_BODY];
    assert_eq!(record.encode_body(&mut output), Ok(39));
    let sub = "é".repeat(32);
    let msg = "é".repeat(2048);
    let record = LogRecord::new([1; 16], 1, 0, Severity::Debug, &sub, &msg).unwrap();
    assert_eq!(record.encode_body(&mut output), Ok(MAX_LOG_BODY));
}

#[test]
fn repeated_produce_encode_parse_retains_identity() {
    let record = LogRecord::new([1; 16], 1, 9, Severity::Debug, "k", "fault\n\0x").unwrap();
    let mut output = [0; HEADER_LEN + MAX_LOG_BODY];
    for id in 1..=10_000 {
        let n = record
            .encode_envelope(id, &[2; 16], &mut output, &Policy::default())
            .unwrap();
        let f = Frame::parse(std::hint::black_box(&output[..n]), &Policy::default()).unwrap();
        let log = LogRecord::from_frame(&f).unwrap();
        assert_eq!(log.correlation_id(), id);
        assert_eq!(log.boot_id(), &[2; 16]);
        assert_eq!(log.record(), &record);
    }
}
