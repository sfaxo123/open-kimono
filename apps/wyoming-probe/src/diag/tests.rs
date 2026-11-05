//! Focused host tests for the diagnostic: marker sequencing, clock/sleep
//! progress, and — the regression Codex asked for — that a correct bounded
//! silent timeout is classified as healthy, not as a hang.

use super::*;
use std::net::TcpListener;

/// Connected (client socket, silent peer) pair over loopback. The peer accepts
/// and never speaks — the leg-2 shape that hung the guest.
fn silent_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let client = TcpStream::connect(addr).expect("connect");
    let (peer, _) = listener.accept().expect("accept");
    (client, peer)
}

#[test]
fn silent_peer_marker_sequence_brackets_every_primitive() {
    let (client, _peer) = silent_pair();
    let mut d = Diag::new(false);
    let cfg = DiagConfig {
        op_timeout: Duration::from_millis(300),
        backoff: Duration::from_millis(10),
        max_retries: 500,
    };
    let out = run(&client, &cfg, &mut d);

    assert_eq!(
        d.phases(),
        vec![
            phase::START,
            phase::CLOCK_A,
            phase::PROBE_SLEEP_BEFORE,
            phase::PROBE_SLEEP_AFTER,
            phase::CLOCK_B,
            phase::NB_BEFORE,
            phase::NB_OK,
            phase::WRITE_BEFORE,
            phase::WRITE_DATA,
            phase::READ_BEFORE,
            phase::READ_WOULDBLOCK,
            phase::RETRY_SLEEP_BEFORE,
            phase::RETRY_SLEEP_AFTER,
            phase::DEADLINE_HIT,
            phase::DONE,
        ],
        "marker sequence: {:?}",
        d.markers()
    );
    assert_eq!(out.write_result, Some(phase::WRITE_DATA));
    assert_eq!(out.read_result, Some(phase::READ_WOULDBLOCK));
    // The deadline (not the cap) must end a 300 ms leg with a 10 ms backoff:
    // only true if every retry really slept. A spin loop would hit the cap in
    // microseconds and look like a dead clock.
    assert!(
        out.retries >= 10,
        "retries are not sleeping: {}",
        out.retries
    );
    assert!(
        d.stamp(phase::DEADLINE_HIT).expect("marked") >= cfg.op_timeout.as_micros(),
        "deadline fired early: the leg cannot have spent its timeout"
    );
}

#[test]
fn bounded_silent_timeout_is_a_healthy_verdict_not_a_hang() {
    // Regression: the raw read DID return (WouldBlock, repeatedly) and the
    // in-process bound fired. Calling that "read never returned" is false.
    let (client, _peer) = silent_pair();
    let mut d = Diag::new(false);
    let cfg = DiagConfig {
        op_timeout: Duration::from_millis(200),
        backoff: Duration::from_millis(10),
        max_retries: 500,
    };
    let out = run(&client, &cfg, &mut d);
    assert_eq!(out.verdict, Verdict::BoundedTimeout);
    assert_eq!(out.verdict.name(), "bounded_silent_timeout_ok");
    assert_eq!(
        out.verdict.exit_code(),
        0,
        "a correct bounded timeout must not exit as a failure"
    );
}

#[test]
fn retry_cap_is_a_distinct_verdict_from_a_bounded_timeout() {
    // The cap firing before the deadline is the dead-clock / mis-sized-cap
    // signal, and must not be conflated with the healthy bounded timeout.
    let (client, _peer) = silent_pair();
    let mut d = Diag::new(false);
    let cfg = DiagConfig {
        op_timeout: Duration::from_secs(60),
        backoff: Duration::from_millis(10),
        max_retries: 3,
    };
    let started = Instant::now();
    let out = run(&client, &cfg, &mut d);
    assert_eq!(out.verdict, Verdict::RetryCapHit);
    assert_eq!(out.verdict.exit_code(), 1);
    assert_eq!(out.retries, 3);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the cap did not bound the returned iterations: {:?}",
        started.elapsed()
    );
}

#[test]
fn clock_and_sleep_markers_show_progress_when_the_primitives_work() {
    // Host control: on a working platform the clock/sleep pairs must show
    // elapsed time, so flatness on the guest is a real signal.
    let (client, _peer) = silent_pair();
    let mut d = Diag::new(false);
    let cfg = DiagConfig {
        op_timeout: Duration::from_millis(200),
        backoff: Duration::from_millis(20),
        max_retries: 500,
    };
    let out = run(&client, &cfg, &mut d);
    assert!(
        out.clock_advanced_us >= cfg.backoff.as_micros(),
        "clock_a→clock_b brackets a {}us sleep, advanced {}us",
        cfg.backoff.as_micros(),
        out.clock_advanced_us
    );
    let before = d.stamp(phase::RETRY_SLEEP_BEFORE).expect("marked");
    let after = d.stamp(phase::RETRY_SLEEP_AFTER).expect("marked");
    assert!(
        after.saturating_sub(before) >= cfg.backoff.as_micros(),
        "retry sleep marker pair shows no elapsed time"
    );
    assert!(
        d.stamp(phase::NB_OK).expect("marked") >= d.stamp(phase::NB_BEFORE).expect("marked"),
        "nonblocking markers went backwards on the clock"
    );
}

#[test]
fn peer_bytes_end_the_read_leg_without_any_retry_markers() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let client = TcpStream::connect(addr).expect("connect");
    let (mut peer, _) = listener.accept().expect("accept");
    peer.write_all(b"pong-frame").expect("peer write");
    // Let the bytes land in the socket buffer before the run starts.
    thread::sleep(Duration::from_millis(50));

    let mut d = Diag::new(false);
    let cfg = DiagConfig {
        op_timeout: Duration::from_secs(5),
        backoff: Duration::from_millis(10),
        max_retries: 16,
    };
    let out = run(&client, &cfg, &mut d);
    assert_eq!(out.verdict, Verdict::ReadAnswered);
    assert_eq!(out.read_result, Some(phase::READ_DATA));
    assert_eq!(out.retries, 0);
    let phases = d.phases();
    assert!(
        !phases.contains(&phase::RETRY_SLEEP_BEFORE),
        "a healthy read must not emit retry markers: {phases:?}"
    );
    assert!(
        !phases.contains(&phase::DEADLINE_HIT),
        "a healthy read must not reach the deadline: {phases:?}"
    );
}

#[test]
fn markers_stay_bounded_however_many_retries_run() {
    let (client, _peer) = silent_pair();
    let mut d = Diag::new(false);
    let cfg = DiagConfig {
        op_timeout: Duration::from_millis(400),
        backoff: Duration::from_millis(2),
        max_retries: 5000,
    };
    let out = run(&client, &cfg, &mut d);
    assert!(
        out.retries > 20,
        "expected many retries, got {}",
        out.retries
    );

    let total = d.markers().len();
    assert!(total <= 16, "marker flood: {total} markers");
    let mut phases = d.phases();
    phases.sort_unstable();
    phases.dedup();
    assert_eq!(phases.len(), total, "a phase marker was emitted twice");
    for (i, m) in d.markers().iter().enumerate() {
        assert_eq!(m.seq, i as u32, "marker numbering broke at {i}");
        if i > 0 {
            assert!(
                m.t_us >= d.markers()[i - 1].t_us,
                "monotonic stamps went backwards at {i}"
            );
        }
    }
}
