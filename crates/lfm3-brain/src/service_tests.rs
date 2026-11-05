use super::*;
use std::sync::atomic::AtomicBool;

const LIM: Limits = Limits {
    request_bytes: 512,
    context_bytes: 64,
    context_tokens: 16,
    output_tokens: 4,
    output_bytes: 32,
};
const GOOD: &[u8] = br#"{"id":"r-1","model":"LiquidAI/LFM3-1.7B","messages":[{"role":"user","content":"hello"}],"max_new_tokens":3}"#;
struct Fixture(Vec<&'static str>);
impl Backend for Fixture {
    fn context_tokens(&self, request: &Request) -> Result<usize, Error> {
        Ok(request
            .messages
            .iter()
            .map(|m| m.content.split_whitespace().count())
            .sum())
    }
    fn stream(
        &self,
        _: &Request,
        emit: &mut dyn FnMut(&str) -> Result<(), Error>,
    ) -> Result<(), Error> {
        for s in &self.0 {
            emit(s)?;
        }
        Ok(())
    }
}
#[derive(Default)]
struct Events(Vec<Event>);
impl Sink for Events {
    fn send(&mut self, e: Event) -> Result<(), Error> {
        self.0.push(e);
        Ok(())
    }
}
struct FailSink;
impl Sink for FailSink {
    fn send(&mut self, _: Event) -> Result<(), Error> {
        Err(Error::Sink)
    }
}
fn run(
    input: &[u8],
    backend: &dyn Backend,
    sink: &mut dyn Sink,
    cancelled: &AtomicBool,
    deadline: Option<Instant>,
) -> Result<(), Error> {
    serve(
        input,
        LIM,
        backend,
        sink,
        &Control {
            cancelled,
            deadline,
        },
    )
}

#[test]
fn streams_ordered_events_and_treats_command_like_text_as_data() {
    let mut out = Events::default();
    let c = AtomicBool::new(false);
    run(
        GOOD,
        &Fixture(vec!["hello ", "<tool>rm -rf /</tool>"]),
        &mut out,
        &c,
        None,
    )
    .unwrap();
    assert!(matches!(out.0.first(), Some(Event::Started { .. })));
    assert!(matches!(out.0[1], Event::Token { index: 0, .. }));
    assert!(matches!(out.0[2], Event::Token { index: 1, .. }));
    assert!(matches!(
        out.0.last(),
        Some(Event::Finished { tokens: 2, .. })
    ));
}
#[test]
fn rejects_invalid_json_model_fields_and_bounds() {
    let c = AtomicBool::new(false);
    let mut out = Events::default();
    assert!(matches!(
        parse(b"{", LIM),
        Err(Error::Invalid("request JSON"))
    ));
    assert!(matches!(
        parse(&[GOOD, &b" ".repeat(LIM.request_bytes)].concat(), LIM),
        Err(Error::Invalid("request byte limit"))
    ));
    let model = String::from_utf8(GOOD.to_vec())
        .unwrap()
        .replace(MODEL_ID, "elsewhere");
    assert!(parse(model.as_bytes(), LIM).is_err());
    let unknown = String::from_utf8(GOOD.to_vec())
        .unwrap()
        .replace("\"max_new_tokens\":3", "\"max_new_tokens\":3,\"extra\":1");
    assert!(parse(unknown.as_bytes(), LIM).is_err());
    let context = String::from_utf8(GOOD.to_vec())
        .unwrap()
        .replace("hello", "x".repeat(65).as_str());
    assert!(parse(context.as_bytes(), LIM).is_err());
    assert!(run(
        GOOD,
        &Fixture(vec!["123456789012345678901234567890123"]),
        &mut out,
        &c,
        None
    )
    .is_err());
}
#[test]
fn unavailable_backend_is_distinct() {
    struct Missing;
    impl Backend for Missing {
        fn context_tokens(&self, _: &Request) -> Result<usize, Error> {
            Err(Error::BackendUnavailable)
        }
        fn stream(
            &self,
            _: &Request,
            _: &mut dyn FnMut(&str) -> Result<(), Error>,
        ) -> Result<(), Error> {
            Err(Error::BackendUnavailable)
        }
    }
    let mut out = Events::default();
    let c = AtomicBool::new(false);
    assert_eq!(
        run(GOOD, &Missing, &mut out, &c, None),
        Err(Error::BackendUnavailable)
    );
    assert!(
        out.0.is_empty(),
        "unavailable backend emitted a success event"
    );
}
#[test]
fn cancellation_deadline_and_sink_failure_stop_streaming() {
    let mut out = Events::default();
    let c = AtomicBool::new(true);
    assert_eq!(
        run(GOOD, &Fixture(vec!["x"]), &mut out, &c, None),
        Err(Error::Cancelled)
    );
    let c = AtomicBool::new(false);
    let mut out = Events::default();
    assert_eq!(
        run(
            GOOD,
            &Fixture(vec!["x"]),
            &mut out,
            &c,
            Some(Instant::now() - std::time::Duration::from_secs(1))
        ),
        Err(Error::Deadline)
    );
    assert_eq!(
        run(GOOD, &Fixture(vec!["x"]), &mut FailSink, &c, None),
        Err(Error::Sink)
    );
}

#[test]
fn token_budget_includes_output_reservation() {
    struct AlmostFull;
    impl Backend for AlmostFull {
        fn context_tokens(&self, _: &Request) -> Result<usize, Error> {
            Ok(14)
        }
        fn stream(
            &self,
            _: &Request,
            _: &mut dyn FnMut(&str) -> Result<(), Error>,
        ) -> Result<(), Error> {
            panic!("generation must not start beyond the total token budget")
        }
    }
    let mut out = Events::default();
    assert_eq!(
        run(GOOD, &AlmostFull, &mut out, &AtomicBool::new(false), None),
        Err(Error::Invalid("context token limit"))
    );
    assert!(out.0.is_empty());
}

#[test]
fn cancellation_between_tokens_has_no_finished_event() {
    struct CancelAfterToken<'a> {
        flag: &'a AtomicBool,
        events: Vec<Event>,
    }
    impl Sink for CancelAfterToken<'_> {
        fn send(&mut self, event: Event) -> Result<(), Error> {
            if matches!(event, Event::Token { .. }) {
                self.flag.store(true, Ordering::Relaxed);
            }
            self.events.push(event);
            Ok(())
        }
    }
    let c = AtomicBool::new(false);
    let mut out = CancelAfterToken {
        flag: &c,
        events: Vec::new(),
    };
    assert_eq!(
        run(GOOD, &Fixture(vec!["first", "second"]), &mut out, &c, None),
        Err(Error::Cancelled)
    );
    assert_eq!(out.events.len(), 2);
    assert!(matches!(
        out.events.last(),
        Some(Event::Token { index: 0, .. })
    ));
}
