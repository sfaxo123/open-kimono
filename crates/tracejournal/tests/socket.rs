#![cfg(unix)]
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn socket_publish_backlog_live_and_invalid_input() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("traced-{}-{unique}.sock", std::process::id()));
    let mut child = Command::new(env!("CARGO_BIN_EXE_traced-linux"))
        .arg(&path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let connect = || {
        (0..100)
            .find_map(|_| {
                let result = UnixStream::connect(&path).ok();
                if result.is_none() {
                    std::thread::sleep(Duration::from_millis(10));
                }
                result
            })
            .unwrap()
    };
    let mut producer = connect();
    producer
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut ack = BufReader::new(producer.try_clone().unwrap());
    let line =
        r#"{"seq":99,"ts_us":1,"service":"voice","payload":{"kind":"op6","type":"mic_started"}}"#;
    writeln!(producer, "{line}").unwrap();
    let mut response = String::new();
    ack.read_line(&mut response).unwrap();
    assert_eq!(response.trim(), r#"{"seq":0}"#);
    let mut consumer = connect();
    consumer
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    writeln!(
        consumer,
        "{{\"backlog\":1,\"subscribe\":{{\"service\":\"voice\"}}}}"
    )
    .unwrap();
    let mut lines = BufReader::new(consumer);
    let mut received = String::new();
    lines.read_line(&mut received).unwrap();
    assert_eq!(trace_events::from_json_line(&received).unwrap().seq, 0);
    writeln!(producer, "{line}").unwrap();
    response.clear();
    ack.read_line(&mut response).unwrap();
    received.clear();
    lines.read_line(&mut received).unwrap();
    assert_eq!(trace_events::from_json_line(&received).unwrap().seq, 1);
    writeln!(producer, "oops").unwrap();
    response.clear();
    ack.read_line(&mut response).unwrap();
    assert!(response.contains("invalid_event"));
    child.kill().unwrap();
    child.wait().unwrap();
    std::fs::remove_file(path).unwrap();
}
