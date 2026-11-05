//! Feed tests: bad lines counted not fatal, partial lines held, file tailing
//! picks up appends.

use std::io::Write;

use timeline::feed::{parse_chunk, Feed};

#[test]
fn parse_chunk_skips_blank_and_counts_bad() {
    let good = r#"{"seq":1,"ts_us":5,"payload":{"kind":"op6","type":"mic_frame","level":0.5}}"#;
    let text = format!("{good}\n\n   \nnot json at all\n{good}\n");
    let batch = parse_chunk(&text);
    assert_eq!(batch.events.len(), 2);
    assert_eq!(batch.bad_lines, 1);
    assert_eq!(batch.events[0].seq, 1);
}

#[test]
fn parse_chunk_rejects_wrong_schema_shapes() {
    // Right JSON, wrong schema: unknown payload kind must be a bad line.
    let batch = parse_chunk(r#"{"seq":1,"ts_us":5,"payload":{"kind":"nope","type":"x"}}"#);
    assert_eq!(batch.events.len(), 0);
    assert_eq!(batch.bad_lines, 1);
}

#[test]
fn file_feed_holds_partial_lines_and_tails_appends() {
    let dir = std::env::temp_dir().join(format!("timeline-feed-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("journal.jsonl");

    let line = r#"{"seq":1,"ts_us":5,"payload":{"kind":"op6","type":"mic_frame","level":0.5}}"#;
    let mut f = std::fs::File::create(&path).unwrap();
    // A full line plus an unterminated partial one.
    writeln!(f, "{line}").unwrap();
    write!(f, "{{\"seq\":2,\"ts_").unwrap();
    f.flush().unwrap();

    let mut feed = Feed::file(&path).unwrap();
    let batch = feed.poll().unwrap();
    assert_eq!(batch.events.len(), 1, "only the complete line parses");
    assert_eq!(batch.bad_lines, 0, "the partial line is held, not counted bad");

    // Complete the partial line, append another, poll again.
    writeln!(f, "us\":9,\"payload\":{{\"kind\":\"op6\",\"type\":\"mic_frame\",\"level\":0.7}}}}").unwrap();
    f.flush().unwrap();
    let batch = feed.poll().unwrap();
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].seq, 2);

    // Non-UTF-8 garbage corrupts at most its own line.
    f.write_all(b"\xff\xfe bad bytes\n").unwrap();
    writeln!(f, "{line}").unwrap();
    f.flush().unwrap();
    let batch = feed.poll().unwrap();
    assert_eq!(batch.bad_lines, 1);
    assert_eq!(batch.events.len(), 1);

    drop(f);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scheme_feed_keeps_complete_records_across_polls() {
    let path = std::env::temp_dir().join(format!("timeline-scheme-{}", std::process::id()));
    let line = r#"{"seq":1,"ts_us":5,"payload":{"kind":"op6","type":"mic_frame","level":0.5}}"#;
    let mut writer = std::fs::File::create(&path).unwrap();
    write!(writer, "{line}\n{line}").unwrap();
    writer.flush().unwrap();
    let mut feed = Feed::scheme(&path).unwrap();
    assert_eq!(feed.poll().unwrap().events.len(), 1);
    writeln!(writer).unwrap();
    writer.flush().unwrap();
    assert_eq!(feed.poll().unwrap().events.len(), 1);
    assert_eq!(feed.poll().unwrap().events.len(), 0);
    drop(writer);
    std::fs::remove_file(path).unwrap();
}
