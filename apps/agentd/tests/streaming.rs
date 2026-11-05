//! CV3 streaming: the brain is asked to stream, complete sentences
//! land in the journal as `answer_chunk` records inside the run, and
//! the failure shapes are honest — a brain that answers whole is
//! `buffered`, content streamed before a tool call is `preamble`, a
//! stream that breaks after a chunk aborts the run (never a second
//! brain's words mixed into the first one's speech), and a stream
//! that breaks before any sentence falls back as before.

mod common;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use agentd::agent::Agent;
use agentd::config::{self, Config};
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;
use common::{chat_body, spawn, text_message, Step};
use trace_events::Event;

fn unique(tag: &str) -> PathBuf {
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!("agentd-cv3-{tag}-{}-{thread}", std::process::id()))
}

fn probe_fixture() -> ProbeRoot {
    let dir = unique("probe");
    fs::create_dir_all(dir.join("sys/class/power_supply/bq27411-0")).expect("fixture battery");
    fs::write(dir.join("sys/class/power_supply/bq27411-0/capacity"), "88\n").unwrap();
    fs::write(dir.join("sys/class/power_supply/bq27411-0/status"), "Discharging\n").unwrap();
    fs::create_dir_all(dir.join("proc/net")).unwrap();
    fs::write(dir.join("proc/uptime"), "3600.5 12000.0\n").unwrap();
    fs::write(dir.join("proc/meminfo"), "MemTotal:  7723008 kB\nMemAvailable:  4800000 kB\n").unwrap();
    fs::write(dir.join("proc/loadavg"), "0.42 0.11 0.05 1/100 1234\n").unwrap();
    fs::write(
        dir.join("proc/net/dev"),
        "Inter-|   Receive\n face |bytes\n    lo: 100 0\n wlan0: 4321 987\n vpn0: 555 666\n",
    )
    .unwrap();
    ProbeRoot(dir)
}

fn key_file() -> PathBuf {
    let path = unique("key");
    fs::write(&path, "test-key-not-a-secret\n").unwrap();
    path
}

fn config_for(gateway_port: u16, local_port: u16) -> Config {
    let text = format!(
        "listen = \"127.0.0.1:8791\"\n\
         gateway_url = \"http://127.0.0.1:{gateway_port}/v1/chat/completions\"\n\
         gateway_model = \"auto\"\n\
         local_url = \"http://127.0.0.1:{local_port}/v1/chat/completions\"\n\
         local_model = \"gemma3-1b\"\n\
         key_file = \"{}\"\n\
         model_timeout_sec = 10\n\
         max_tool_rounds = 4\n",
        key_file().display()
    );
    config::load(&text).expect("test config must load")
}

fn agent_with(gateway_port: u16, local_port: u16) -> (Agent, Journal) {
    let journal = Journal::new(512);
    let agent = Agent::with_root(config_for(gateway_port, local_port), journal.clone(), probe_fixture());
    (agent, journal)
}

fn lines(events: &[Event]) -> Vec<String> {
    events.iter().filter_map(|e| trace_events::to_json_line(e).ok()).collect()
}

fn chunk_texts(events: &[Event]) -> Vec<String> {
    lines(events)
        .iter()
        .filter(|l| l.contains("\"answer_chunk\""))
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("journal line is json");
            v["payload"]["value"]["text"].as_str().unwrap_or("").to_string()
        })
        .collect()
}

fn stream_modes(events: &[Event]) -> Vec<String> {
    lines(events)
        .iter()
        .filter(|l| l.contains("\"answer_stream\""))
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).expect("journal line is json");
            v["payload"]["value"]["mode"].as_str().unwrap_or("").to_string()
        })
        .collect()
}

// ------------------------------------------------------------ SSE fake brain

/// One scripted connection: a list of SSE data payloads, then either
/// a clean chunked end (`drop = false`) or an abrupt close mid-body.
struct SseStep {
    payloads: Vec<String>,
    drop: bool,
}

struct SseBrain {
    port: u16,
    seen: Arc<Mutex<Vec<String>>>,
}

fn content_delta(s: &str) -> String {
    serde_json::json!({"choices": [{"index": 0, "delta": {"content": s}, "finish_reason": null}]})
        .to_string()
}

fn tool_delta(id: &str, name: &str, args_frag: &str) -> String {
    serde_json::json!({"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": id, "type": "function", "function": {"name": name, "arguments": args_frag}}]}, "finish_reason": null}]})
        .to_string()
}

fn finish_delta(reason: &str) -> String {
    serde_json::json!({"choices": [{"index": 0, "delta": {}, "finish_reason": reason}]}).to_string()
}

fn spawn_sse(script: Vec<SseStep>) -> SseBrain {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind sse brain");
    let port = listener.local_addr().expect("local addr").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_thread = Arc::clone(&seen);
    thread::spawn(move || {
        for step in script {
            let (mut stream, _) = match listener.accept() {
                Ok(s) => s,
                Err(_) => return,
            };
            let body = read_request_body(&mut stream);
            seen_thread.lock().unwrap().push(body);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            );
            for payload in &step.payloads {
                let frame = format!("data: {payload}\n\n");
                if stream
                    .write_all(format!("{:x}\r\n{frame}\r\n", frame.len()).as_bytes())
                    .is_err()
                {
                    break;
                }
                let _ = stream.flush();
                thread::sleep(Duration::from_millis(30));
            }
            if step.drop {
                drop(stream); // no terminating chunk, no [DONE]: a cut stream
            } else {
                let _ = stream.write_all(b"0\r\n\r\n");
                let _ = stream.flush();
            }
        }
    });
    SseBrain { port, seen }
}

fn read_request_body(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if stream.read(&mut byte).unwrap_or(0) == 0 {
            return String::new();
        }
        buf.extend_from_slice(&byte);
        if buf.len() > 200_000 {
            return String::new();
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let length = head
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
        .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().parse::<usize>().unwrap_or(0)))
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    let mut scratch = [0u8; 4096];
    while body.len() < length {
        match stream.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(n) => body.extend_from_slice(&scratch[..n]),
        }
    }
    String::from_utf8_lossy(&body).to_string()
}

// ------------------------------------------------------------------ tests

#[test]
fn a_streamed_answer_journals_sentence_chunks_inside_the_run() {
    let brain = spawn_sse(vec![
        SseStep {
            payloads: vec![
                tool_delta("call-1", "phone_status", "{}"),
                finish_delta("tool_calls"),
                "[DONE]".to_string(),
            ],
            drop: false,
        },
        SseStep {
            payloads: vec![
                content_delta("Maps is open with directions to San "),
                content_delta("Francisco. The drive takes about "),
                content_delta("2.5 hours via I-80. "),
                content_delta("Drive safely"),
                finish_delta("stop"),
                "[DONE]".to_string(),
            ],
            drop: false,
        },
    ]);
    let (agent, journal) = agent_with(brain.port, 1);
    let summary = agent.handle("open maps and give me directions to San Francisco");
    let answer = "Maps is open with directions to San Francisco. The drive takes about 2.5 hours via I-80. Drive safely";
    assert_eq!(summary.answer.as_deref(), Some(answer), "summary: {summary:?}");
    assert_eq!(summary.error, None);

    let events = journal.snapshot();
    let chunks = chunk_texts(&events);
    assert_eq!(
        chunks,
        vec![
            "Maps is open with directions to San Francisco.".to_string(),
            "The drive takes about 2.5 hours via I-80.".to_string(),
            "Drive safely".to_string(),
        ],
        "chunks must be the answer's sentences, in order"
    );
    assert_eq!(chunks.join(" "), answer, "the chunks reassemble into the answer");
    assert_eq!(stream_modes(&events), vec!["streamed".to_string()]);

    let ls = lines(&events);
    let finished = ls.iter().find(|l| l.contains("\"RUN_FINISHED\"")).expect("run finished");
    assert!(finished.contains("\"streamed\":true"), "run_finished records the stream: {finished}");
    assert!(finished.contains("\"chunks\":3"), "run_finished records the chunk count: {finished}");
    let last_chunk_pos = ls.iter().rposition(|l| l.contains("\"answer_chunk\"")).unwrap();
    let finished_pos = ls.iter().position(|l| l.contains("\"RUN_FINISHED\"")).unwrap();
    assert!(last_chunk_pos < finished_pos, "chunks land inside the run, before RUN_FINISHED");

    let bodies = brain.seen.lock().unwrap().clone();
    assert_eq!(bodies.len(), 2, "one call per round");
    assert!(bodies.iter().all(|b| b.contains("\"stream\":true")), "every brain call asks to stream: {bodies:?}");
}

#[test]
fn the_answer_tools_text_streams_out_of_its_arguments() {
    let brain = spawn_sse(vec![SseStep {
        payloads: vec![
            tool_delta("call-1", "answer", "{\"text\": \"Maps is open. The drive"),
            tool_delta("", "", " takes about an hour.\"}"),
            finish_delta("tool_calls"),
            "[DONE]".to_string(),
        ],
        drop: false,
    }]);
    let (agent, journal) = agent_with(brain.port, 1);
    let summary = agent.handle("open maps and give me directions to San Francisco");
    assert_eq!(summary.answer.as_deref(), Some("Maps is open. The drive takes about an hour."), "summary: {summary:?}");

    let events = journal.snapshot();
    assert_eq!(
        chunk_texts(&events),
        vec!["Maps is open.".to_string(), "The drive takes about an hour.".to_string()],
        "the answer tool's text is chunked out of its streaming arguments"
    );
    assert_eq!(stream_modes(&events), vec!["streamed".to_string()]);
}

#[test]
fn content_streamed_before_a_tool_call_is_journaled_as_preamble() {
    let brain = spawn_sse(vec![
        SseStep {
            payloads: vec![
                content_delta("Let me check the battery for you. "),
                tool_delta("call-1", "phone_status", "{}"),
                finish_delta("tool_calls"),
                "[DONE]".to_string(),
            ],
            drop: false,
        },
        SseStep {
            payloads: vec![
                content_delta("Battery is at 88 percent."),
                finish_delta("stop"),
                "[DONE]".to_string(),
            ],
            drop: false,
        },
    ]);
    let (agent, journal) = agent_with(brain.port, 1);
    let summary = agent.handle("how is the battery");
    assert_eq!(summary.answer.as_deref(), Some("Battery is at 88 percent."), "the answer is the final round's, not the preamble");

    let events = journal.snapshot();
    assert_eq!(stream_modes(&events), vec!["preamble".to_string(), "streamed".to_string()]);
    assert_eq!(
        chunk_texts(&events),
        vec!["Let me check the battery for you.".to_string(), "Battery is at 88 percent.".to_string()],
        "both were spoken-shaped; the journal's modes say which was the answer"
    );
}

#[test]
fn a_brain_that_answers_whole_is_buffered_not_streamed() {
    let brain = spawn(
        "gateway",
        vec![Step::Reply(chat_body(
            text_message("Battery is at 88 percent. It is discharging."),
            "auto",
        ))],
    );
    let (agent, journal) = agent_with(brain.port, 1);
    let summary = agent.handle("how is the battery");
    assert_eq!(summary.answer.as_deref(), Some("Battery is at 88 percent. It is discharging."));

    let events = journal.snapshot();
    assert!(chunk_texts(&events).is_empty(), "no chunks for a buffered answer");
    assert_eq!(stream_modes(&events), vec!["buffered".to_string()]);
    let ls = lines(&events);
    let finished = ls.iter().find(|l| l.contains("\"RUN_FINISHED\"")).expect("run finished");
    assert!(finished.contains("\"streamed\":false"), "{finished}");
}

#[test]
fn a_stream_that_breaks_after_a_chunk_aborts_the_run() {
    let brain = spawn_sse(vec![SseStep {
        payloads: vec![content_delta("Partial answer here. "), content_delta("More partial")],
        drop: true,
    }]);
    let (agent, journal) = agent_with(brain.port, 1);
    let summary = agent.handle("tell me something");
    assert!(summary.answer.is_none(), "a cut answer is never returned as whole: {summary:?}");
    let err = summary.error.expect("the run must fail");
    assert!(err.contains("stream broke"), "the error names the break: {err}");

    let events = journal.snapshot();
    assert_eq!(chunk_texts(&events), vec!["Partial answer here.".to_string()], "the spoken chunk is journaled honestly");
    assert_eq!(stream_modes(&events), vec!["aborted".to_string()]);
    let ls = lines(&events);
    assert!(ls.iter().any(|l| l.contains("\"RUN_ERROR\"")), "the run errors");
    assert!(!ls.iter().any(|l| l.contains("\"RUN_FINISHED\"")), "the run never finishes with the partial answer");
}

#[test]
fn a_stream_that_breaks_before_any_sentence_falls_back() {
    let gateway = spawn_sse(vec![SseStep {
        payloads: vec![content_delta("Partial text with no terminator")],
        drop: true,
    }]);
    let local = spawn("local", vec![Step::Reply(chat_body(text_message("Fallback answer."), "gemma3-1b"))]);
    let (agent, journal) = agent_with(gateway.port, local.port);
    let summary = agent.handle("tell me something");
    assert_eq!(summary.answer.as_deref(), Some("Fallback answer."), "summary: {summary:?}");

    let events = journal.snapshot();
    assert!(chunk_texts(&events).is_empty(), "nothing was journaled as speech from the dead stream");
    assert!(!stream_modes(&events).contains(&"aborted".to_string()), "no abort when nothing was spoken");
}

#[test]
fn a_caller_supplied_run_id_is_adopted() {
    let brain = spawn_sse(vec![SseStep {
        payloads: vec![content_delta("Hello there."), finish_delta("stop"), "[DONE]".to_string()],
        drop: false,
    }]);
    let (agent, journal) = agent_with(brain.port, 1);
    let summary = agent.handle_session_conf_rid(
        "say hello",
        None,
        None,
        false,
        None,
        Some("run-v1234567890"),
    );
    assert_eq!(summary.run_id, "run-v1234567890");
    assert_eq!(summary.answer.as_deref(), Some("Hello there."));
    let events = journal.snapshot();
    let ls = lines(&events);
    assert!(
        ls.iter().any(|l| l.contains("\"RUN_STARTED\"") && l.contains("run-v1234567890")),
        "the run journals under the caller's id"
    );
    assert_eq!(chunk_texts(&events), vec!["Hello there.".to_string()]);
}
