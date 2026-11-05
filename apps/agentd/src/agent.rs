//! The request loop: text in, model call, tool calls, results back, answer out
//! — with every step written to the journal as it happens.
//!
//! The fallback chain lives here too, and it is never silent: a brain that
//! fails gets a failed span and a `handoff` event naming the reason, and if
//! every brain fails the run ends with an explicit "can't answer, here's why"
//! `RUN_ERROR` rather than an empty answer.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{json, Value};
use trace_events::{HandoffStatus, SpanStatus};

use crate::broker::{self, Broker, Decision};
use crate::config::{Brain, Config};
use crate::journal::Journal;
use crate::kimono::Recorder;
use crate::model;
use crate::phone::ProbeRoot;
use crate::repair::{self, PendingRepair, Stratum};
use crate::session::{SessionContext, SessionStore};
use crate::smoke::now_us;
use crate::tools;

/// What one request produced, for the HTTP layer to return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunSummary {
    pub run_id: String,
    pub answer: Option<String>,
    pub error: Option<String>,
    pub brain: Option<String>,
}

pub struct Agent {
    pub config: Config,
    pub journal: Journal,
    pub root: ProbeRoot,
    /// The Lego layer: one live svcd executor behind the four service tools.
    pub services: crate::services::ServiceHub,
    /// What is on, who needs it, and what should be off (A5).
    pub power: crate::power::PowerManager,
    /// The capability broker every tool call is gated through (A1.2).
    pub broker: Broker,
    /// Conversation sessions (CV1): state for the spoken conversations
    /// currently alive, keyed by the caller's session id.
    pub sessions: SessionStore,
    /// Commit-class calls that were refused for confirmation, keyed by the
    /// label the user was shown. Confirming that label installs exactly
    /// these arguments, even if the model re-authors the call with a
    /// different id on the confirming run (the label the user approved is
    /// the contract, not whatever the model writes the second time).
    held_commits: std::sync::Mutex<std::collections::HashMap<String, (String, Value)>>,
    runs: AtomicU64,
}

/// What one run established, handed to the session record (CV1): the
/// entities its successful tool calls set, and the slots it left open.
#[derive(Default)]
struct RunFacts {
    entities: Vec<(String, String)>,
    pending: Vec<String>,
    /// CV2: the repair state to store when the turn ends — `Some`
    /// when this turn asked the targeted question, `None` when no
    /// repair is open after this turn (including the turn that
    /// consumed one: a repair is never left open twice).
    repair: Option<PendingRepair>,
    /// CV2: the text to record as the user's turn when it differs
    /// from the wire text — the composed request of a consumed
    /// repair, so the history the next turn replays is the request
    /// the brain actually served.
    effective_user: Option<String>,
}

impl Agent {
    pub fn new(config: Config, journal: Journal) -> Agent {
        Agent::with_root(config, journal, ProbeRoot::system())
    }

    /// Point the read-only phone probes somewhere else (host tests use a
    /// fixture tree instead of the developer's own `/proc`).
    pub fn with_root(config: Config, journal: Journal, root: ProbeRoot) -> Agent {
        let services = crate::services::ServiceHub::start(journal.clone(), crate::services::seed_sensors(&root));
        let power = crate::power::PowerManager::new(
            journal.clone(),
            root.clone(),
            std::time::Duration::from_secs(config.idle_timeout_sec),
            config.local_model_port(),
        );
        power.set_preapproved(config.power_preapproved.iter().cloned());
        Agent { config, journal, root, services, power, broker: Broker::new(), sessions: SessionStore::new(), held_commits: Default::default(), runs: AtomicU64::new(0) }
    }

    /// The instructions every request starts with. The Lego section carries the
    /// ServiceIR shape and two examples that are known to validate and pass
    /// their own simulation (see `ir_help`), because a model that guesses the
    /// shape wastes a round on the validator. The BRAINFIX section
    /// (see `brainfix`) carries the confirmation discipline for inferred
    /// destinations, adopted verbatim from the key-tests workbench.
    pub fn system_prompt(&self) -> String {
        let tools: Vec<String> = tools::definitions()
            .iter()
            .filter_map(|d| d["function"]["name"].as_str().map(str::to_string))
            .collect();
        format!(
            "You are agentd, the agentic layer on a OnePlus 6 phone.\n\
             You can call tools. Always finish a request by calling the `answer` tool with a short,\n\
             spoken-friendly sentence; if you used tools, say what you found and what you did.\n\
             Tools you have: {}.\n\
             Never invent a tool. Never claim you changed something you did not.\n\
             Acting is a tool call, never a promise: when the user asks for something to be created,\n\
             started or stopped, call the tool that does it in this turn instead of replying with what\n\
             you are about to do. If a tool fails, call it again or call `answer` with the failure.\n\
             If a tool result says `ask_to_commit`, do not retry the tool in this turn: call `answer`,\n\
             name the label it gives, and say the user can allow exactly that by repeating the\n\
             request with that confirmation. Never claim the blocked action happened.\n\
             If you cannot do something, call `answer` and say so plainly, with the reason.{}{}{}\n",
            tools.join(", "),
            crate::ir_help::section(),
            crate::brainfix::section(),
            crate::brainfix::addendum_section()
        )
    }

    /// Run one request end to end. Blocks until the answer or an error; every
    /// step is already in the journal by the time this returns.
    /// One request, start to finish. The power manager is told when it begins
    /// and when it ends, so the idle clock and the "a request needs this"
    /// answer are both real and not guessed later.
    pub fn handle(&self, text: &str) -> RunSummary {
        self.handle_with_confirm(text, None)
    }

    /// One request, with the caller's explicit confirmation for at most
    /// one commit-class label (`{"confirm": "<label>"}` on the wire). A
    /// commit whose label is not exactly this string does not run.
    pub fn handle_with_confirm(&self, text: &str, confirm: Option<&str>) -> RunSummary {
        self.handle_session(text, confirm, None, false)
    }

    /// One request inside a conversation session (CV1). The session's
    /// earlier turns and entities reach the brain as context (data,
    /// under the untouched system prompt — see `session`), and this
    /// turn is recorded back into the session when it finishes.
    /// `follow_up` marks a turn the listener accepted without the wake
    /// word inside the follow-up window; it is journaled, never hidden.
    pub fn handle_session(&self, text: &str, confirm: Option<&str>, session: Option<&str>, follow_up: bool) -> RunSummary {
        self.handle_session_conf(text, confirm, session, follow_up, None)
    }

    /// One request, with the caller's ASR confidence for the text
    /// (CV2): the wire's transcription-confidence value when the
    /// caller has one, `None` when it has no signal. The strata in
    /// `repair` decide what the value changes; `None` changes
    /// nothing at all.
    pub fn handle_session_conf(
        &self,
        text: &str,
        confirm: Option<&str>,
        session: Option<&str>,
        follow_up: bool,
        asr_confidence: Option<f32>,
    ) -> RunSummary {
        self.handle_session_conf_rid(text, confirm, session, follow_up, asr_confidence, None)
    }

    /// One request, with a caller-minted run id to adopt (CV3): the
    /// caller (voiced) starts tailing the journal before it POSTs,
    /// so it must know the run's id in advance. The id is adopted
    /// verbatim; absent, one is minted as before. Adoption changes
    /// nothing else about the run.
    pub fn handle_session_conf_rid(
        &self,
        text: &str,
        confirm: Option<&str>,
        session: Option<&str>,
        follow_up: bool,
        asr_confidence: Option<f32>,
        run_id: Option<&str>,
    ) -> RunSummary {
        self.power.request_started();
        let summary = self.run(text, confirm, session, follow_up, asr_confidence, run_id);
        self.power.request_finished();
        self.power.tick(Some(&self.services));
        summary
    }

    fn run(&self, text: &str, confirm: Option<&str>, session: Option<&str>, follow_up: bool, asr_confidence: Option<f32>, run_id: Option<&str>) -> RunSummary {
        match session {
            Some(id) => {
                let ctx = self.sessions.begin(id, std::time::Instant::now());
                let (summary, mut facts) = self.run_inner(text, confirm, Some((id, follow_up, &ctx)), asr_confidence, run_id);
                if let Some(answer) = &summary.answer {
                    if answer.trim_end().ends_with('?') {
                        facts.pending.push("answer_question".to_string());
                    }
                }
                // A consumed repair records its composed request as
                // the user's turn; anything else records the wire text.
                let record_text = facts.effective_user.take().unwrap_or_else(|| text.to_string());
                let repair = facts.repair.take();
                self.sessions.record(id, &record_text, summary.answer.clone(), facts.entities, facts.pending, std::time::Instant::now());
                self.sessions.set_repair(id, repair, std::time::Instant::now());
                // Session card: emitted after every recorded turn, from
                // the store's new state, so kimono's card updates as
                // the session changes. A snapshot that vanished (TTL
                // raced) emits nothing rather than a stale card.
                if let Some(snap) = self.sessions.snapshot(id, std::time::Instant::now()) {
                    let (manifest, inputs) =
                        crate::modules::session_card(&snap.id, snap.turn_count, snap.follow_up_open, &snap.pending);
                    if crate::modules::validate_manifest(&manifest).is_empty()
                        && crate::modules::missing_required_inputs(&manifest, &inputs).is_empty()
                    {
                        let rec = Recorder::new(self.journal.clone(), &summary.run_id);
                        rec.module_manifest(manifest, inputs);
                    }
                }
                summary
            }
            None => self.run_inner(text, confirm, None, asr_confidence, run_id).0,
        }
    }

    fn run_inner(
        &self,
        text: &str,
        confirm: Option<&str>,
        session: Option<(&str, bool, &SessionContext)>,
        asr_confidence: Option<f32>,
        run_id: Option<&str>,
    ) -> (RunSummary, RunFacts) {
        let n = self.runs.fetch_add(1, Ordering::SeqCst);
        let run_id = match run_id {
            Some(adopted) => adopted.to_string(),
            None => format!("run-{}-{}", now_us(), n),
        };
        let mut rec = Recorder::new(self.journal.clone(), &run_id);
        let mut facts = RunFacts::default();
        match session {
            Some((id, fu, _)) => rec.run_started("phone", &run_id, json!({"text": text, "session": id, "follow_up": fu })),
            None => rec.run_started("phone", &run_id, json!({"text": text })),
        };
        if let Some((id, fu, ctx)) = session {
            rec.session_context(id, ctx.turns.len(), &ctx.entities, &ctx.pending, fu, ctx.fresh);
        }

        // CV2 repair strata: what transcription confidence changes
        // about this turn, decided BEFORE the brain is built or
        // called, and journaled whatever the outcome. Composition
        // with BRAINFIX is by construction: the strata never touch
        // the system prompt, never confirm anything, and a LOW turn
        // cannot act at all — BRAINFIX's must_confirm cases and the
        // broker gate see exactly the turns they saw before CV2.
        let mut effective_text = text.to_string();
        let mut medium_note: Option<String> = None;
        let open_repair = session.and_then(|(_, _, ctx)| ctx.repair.clone());
        if let Some(pending) = open_repair {
            // This turn answers an open repair: compose the original
            // request from the spoken answer and serve THAT. The
            // answer's own confidence is never classified — a repair
            // asks exactly one question, and the answer is accepted
            // as spoken even if it arrives quiet too.
            let composed = repair::compose_resume(&pending, text);
            rec.repair_resumed(&pending.original, &pending.span, text, &composed);
            facts.effective_user = Some(composed.clone());
            effective_text = composed;
        } else {
            let (stratum, reason) = repair::classify(asr_confidence, self.config.repair_high, self.config.repair_low);
            let (kind, span) = repair::extract_span(text);
            rec.repair_stratum(stratum.name(), asr_confidence, &reason, Some(&span));
            match stratum {
                Stratum::High => {}
                Stratum::Medium => {
                    medium_note = Some(repair::medium_note(&span));
                }
                Stratum::Low => {
                    // No brain call, no tool, no broker: the whole
                    // turn is the one targeted question. The repair
                    // is stored for the answer to resume (a
                    // sessionless LOW — only reachable from the API,
                    // since voice turns always carry a session —
                    // stores nothing and says so in the record).
                    let question = repair::question_for(kind, &span);
                    facts.repair = Some(PendingRepair { original: text.to_string(), span, kind });
                    rec.answer(&run_id, &question);
                    rec.run_finished(
                        "phone",
                        &run_id,
                        json!({"answer": question, "repair_stratum": "low", "repair_stored": session.is_some()}),
                    );
                    return (RunSummary { run_id, answer: Some(question), error: None, brain: None }, facts);
                }
            }
        }
        // Bring the service clock up to date first, so a question like "did my
        // reminder fire?" is answered from a journal that already contains it.
        self.services.tick();

        // The system prompt is built exactly as for a sessionless
        // request: session state never edits it (the BRAINFIX discipline
        // is part of it). History, when a session is live, is data
        // replayed as the conversation it is — ordinary user/assistant
        // messages — and the entity summary rides as a user-role
        // background note marked as already acted on, not a new request.
        let mut messages = vec![json!({"role": "system", "content": self.system_prompt()})];
        if let Some((_, _, ctx)) = session {
            for turn in &ctx.turns {
                if let Some(answer) = &turn.answer {
                    messages.push(json!({"role": "user", "content": turn.user}));
                    messages.push(json!({"role": "assistant", "content": answer}));
                }
            }
            if !ctx.entities.is_empty() {
                let established: Vec<String> = ctx.entities.iter().map(|(k, v)| format!("{k}: {v}")).collect();
                messages.push(json!({
                    "role": "user",
                    "content": format!(
                        "Background from earlier in this conversation (already acted on; context for the request that follows, not a new request): {}.",
                        established.join("; ")
                    ),
                }));
            }
        }
        if let Some(note) = &medium_note {
            messages.push(json!({"role": "user", "content": note}));
        }
        messages.push(json!({"role": "user", "content": effective_text}));

        for round in 0..self.config.max_tool_rounds {
            let step = format!("model_round_{}", round + 1);
            rec.step(&step, false);

            let reply = match self.call_brains(&mut rec, &messages, &run_id) {
                Ok(reply) => reply,
                Err(reasons) => {
                    let why = reasons.join(" | ");
                    rec.run_error(&format!("can't answer, here's why: {why}"), Some("no_brain"));
                    return (RunSummary { run_id, answer: None, error: Some(why), brain: None }, facts);
                }
            };

            if reply.tool_calls.is_empty() {
                // A model that answers in prose instead of calling `answer` is
                // still answering; record it as the answer rather than looping.
                let answer = reply.content.clone().unwrap_or_default();
                if answer.trim().is_empty() {
                    rec.run_error(
                        "can't answer, here's why: the model returned neither text nor a tool call",
                        Some("empty_reply"),
                    );
                    return (
                        RunSummary {
                            run_id,
                            answer: None,
                            error: Some("empty reply from model".to_string()),
                            brain: Some(reply.brain_name),
                        },
                        facts,
                    );
                }
                rec.answer(&run_id, &answer);
                rec.run_finished("phone", &run_id, json!({"answer": answer, "brain": reply.brain_name, "streamed": reply.streamed, "chunks": reply.chunks}));
                return (RunSummary { run_id, answer: Some(answer), error: None, brain: Some(reply.brain_name) }, facts);
            }

            messages.push(assistant_tool_calls_message(&reply));
            for (i, call) in reply.tool_calls.iter().enumerate() {
                let call_id = if call.id.is_empty() { format!("{run_id}-call-{}", i + 1) } else { call.id.clone() };
                rec.tool_agui(&call_id, &call.name, &call.arguments, Some(&run_id));
                let span = rec.tool_start(&call.name, &call_id, &call.arguments, None);

                let ctx = tools::ToolCtx::new(&self.root, Some(&self.services), Some(&self.power));
                // Every tool call passes the broker gate (A1.2/A1.GATE). An
                // unconfirmed commit is not executed at all: the journal
                // carries `ask_to_commit` and the model reads why.
                let mut gated = false;
                let outcome = match tools::parse_arguments(&call.arguments) {
                    Ok(mut args) => {
                        // A confirmed label replays the call the user was
                        // shown, not a re-authored one (see held_commits).
                        if call.name != "app_launch" {
                            if let Some(c) = confirm {
                                let held = self.held_commits.lock().unwrap().get(c).cloned();
                                if let Some((tool, held_args)) = held {
                                    if tool == call.name && broker::label_for(&call.name, &args) != c {
                                        args = held_args;
                                    }
                                }
                            }
                        }
                        let label = broker::label_for(&call.name, &args);
                        let confirmed = confirm == Some(label.as_str());
                        // Navigation is destination-bound: an inferred
                        // destination launches only on its own
                        // confirmation (see broker::navigation_gate).
                        // Every other tool keeps the class gate.
                        let decision = if call.name == "app_launch" {
                            let destination = args.get("destination").and_then(Value::as_str).unwrap_or("");
                            let (prior_users, prior_answers, established) = match session {
                                Some((_, _, ctx)) => (
                                    ctx.turns.iter().map(|t| t.user.as_str()).collect::<Vec<_>>(),
                                    ctx.turns.iter().map(|t| t.answer.as_deref().unwrap_or("")).collect::<Vec<_>>(),
                                    ctx.entities.get("destination").map(String::as_str),
                                ),
                                None => (Vec::new(), Vec::new(), None),
                            };
                            broker::navigation_gate(broker::NavGate {
                                destination,
                                current_user: &effective_text,
                                prior_users: &prior_users,
                                prior_answers: &prior_answers,
                                established,
                                wire_confirmed: confirmed,
                            })
                        } else {
                            self.broker.gate(broker::action_class_for(&call.name), &label, confirmed)
                        };
                        match decision {
                            Decision::AskToCommit(label) => {
                                gated = true;
                                facts.pending.push(label.clone());
                                rec.ask_to_commit(&call.name, &label);
                                if call.name != "app_launch" {
                                    self.held_commits.lock().unwrap().insert(label.clone(), (call.name.clone(), args.clone()));
                                }
                                let reason = if call.name == "app_launch" {
                                    format!("`{label}` needs the owner's spoken confirmation of exactly that destination first — the tool was not run. Name that destination, ask \"head there?\", and wait for the owner to confirm that destination; a different destination never confirms it. Never claim the blocked navigation happened.")
                                } else {
                                    format!("`{label}` changes the phone and needs confirmation first — the tool was not run. Tell the user they can allow exactly this by repeating the request with \"confirm\": \"{label}\".")
                                };
                                Ok(tools::ToolOutcome {
                                    content: json!({
                                        "ask_to_commit": label,
                                        "tool": call.name,
                                        "reason": reason,
                                    }),
                                    terminal: false,
                                    answer_text: None,
                                })
                            }
                            _ => {
                                if confirmed {
                                    self.held_commits.lock().unwrap().remove(&label);
                                }
                                let outcome = tools::execute(&call.name, &args, &ctx);
                                // A successful app_launch establishes the
                                // session's destination entity (CV1): later
                                // turns resolve "there" against it.
                                if outcome.is_ok() && call.name == "app_launch" {
                                    if let Some(dest) = args.get("destination").and_then(Value::as_str) {
                                        facts.entities.push(("destination".to_string(), dest.to_string()));
                                    }
                                }
                                outcome
                            }
                        }
                    }
                    Err(e) => Err(e),
                };
                match outcome {
                    Ok(outcome) => {
                        let content_text = outcome.content.to_string();
                        // A gated call did not execute: its span must not
                        // claim a successful tool run.
                        rec.tool_end(span, &call.name, if gated { SpanStatus::Error } else { SpanStatus::Ok }, &content_text, None);
                        rec.tool_result(&run_id, &call_id, outcome.content.clone());
                        // Destination card: only a real, ungated
                        // app_launch success emits one, built from the
                        // actual tool result (app + uri) and the call's
                        // destination. A gated or failed launch emits
                        // nothing — no card for something that did not open.
                        if !gated && call.name == "app_launch" {
                            if let Ok(call_args) = tools::parse_arguments(&call.arguments) {
                                let dest = call_args.get("destination").and_then(Value::as_str).unwrap_or("");
                                let app_arg = call_args.get("app").and_then(Value::as_str).unwrap_or("");
                                let app = outcome.content.get("app").and_then(Value::as_str).unwrap_or(app_arg);
                                let uri = outcome.content.get("uri").and_then(Value::as_str).unwrap_or("");
                                if !dest.is_empty() && !app.is_empty() && !uri.is_empty() {
                                    let (manifest, inputs) = crate::modules::destination_card(dest, app, uri, &run_id);
                                    if crate::modules::validate_manifest(&manifest).is_empty()
                                        && crate::modules::missing_required_inputs(&manifest, &inputs).is_empty()
                                    {
                                        rec.module_manifest(manifest, inputs);
                                    }
                                }
                            }
                        }
                        messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": content_text}));
                        if outcome.terminal {
                            let answer = outcome.answer_text.unwrap_or_default();
                            rec.answer(&run_id, &answer);
                            rec.run_finished("phone", &run_id, json!({"answer": answer, "brain": reply.brain_name, "streamed": reply.streamed, "chunks": reply.chunks}));
                            return (
                                RunSummary {
                                    run_id,
                                    answer: Some(answer),
                                    error: None,
                                    brain: Some(reply.brain_name),
                                },
                                facts,
                            );
                        }
                    }
                    Err(err) => {
                        // The model sees its own mistake and gets another turn.
                        let reason = err.reason();
                        rec.tool_end(span, &call.name, SpanStatus::Error, &reason, None);
                        rec.tool_result(&run_id, &call_id, json!({"error": reason}));
                        messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": reason}));
                    }
                }
            }
            rec.step(&step, true);
        }

        let why = format!("tool round limit reached after {} rounds without an answer", self.config.max_tool_rounds);
        rec.run_error(&format!("can't answer, here's why: {why}"), Some("round_limit"));
        (RunSummary { run_id, answer: None, error: Some(why), brain: None }, facts)
    }

    /// Try each configured brain in order. The first that answers wins; every
    /// failure is journaled with its reason.
    ///
    /// CV3: brains are asked to stream (`stream: true`). Complete
    /// sentences are journaled as `answer_chunk` records the moment
    /// they land — prose content directly, and the `answer` tool's
    /// text decoded out of its streaming arguments — so a listener
    /// can start speaking before the answer finishes. The fallback
    /// rule gains one clause: a brain that fails *before any chunk
    /// was journaled* falls through to the next brain exactly as
    /// before, but a stream that breaks after chunks were journaled
    /// aborts the run — falling through would mix a second brain's
    /// answer into speech the user may already have heard, and the
    /// journal would lie about whose words they were. A brain that
    /// answers without streaming is fine: its shape is journaled as
    /// `buffered` and the whole answer lands at the end, as before.
    /// Content streamed in a round that ends in tool calls (other
    /// than a valid `answer` call) is journaled as `preamble`, so a
    /// reader can always tell speech-that-was-the-answer from
    /// speech-that-preceded it.
    fn call_brains(&self, rec: &mut Recorder, messages: &[Value], run_id: &str) -> Result<Reply, Vec<String>> {
        let mut failures = Vec::new();
        for (index, brain) in self.config.brains.iter().enumerate() {
            let key = if brain.needs_key {
                match crate::config::read_key(&self.config.key_file) {
                    Ok(k) => Some(k),
                    Err(e) => {
                        failures.push(format!("{}: {e}", brain.name));
                        rec.handoff(&brain.name, &format!("skipped, {e}"), HandoffStatus::Failed);
                        continue;
                    }
                }
            } else {
                None
            };

            if index > 0 && !failures.is_empty() {
                rec.handoff(
                    &brain.name,
                    &format!("{} unreachable, using {}", brain_chain_label(&self.config.brains[..index]), brain.name),
                    HandoffStatus::Started,
                );
            }

            let (span, _) = rec.chat_start(&brain.model, provider_of(brain), run_id);
            let payload = json!({
                "model": brain.model,
                "messages": messages,
                "tools": tools::definitions(),
                "tool_choice": "auto",
                "temperature": 0,
                "max_tokens": self.config.max_completion_tokens,
                "stream": true,
            });
            // The sentence sink journals each chunk the moment the
            // stream completes it, counting the two sources apart.
            // The sink's borrows end with the call.
            let mut content_chunks: Vec<String> = Vec::new();
            let mut answer_chunks: Vec<String> = Vec::new();
            let attempt = {
                let mut sink = |source: model::SentenceSource, sentence: &str| {
                    let idx = content_chunks.len() + answer_chunks.len();
                    rec.answer_chunk(run_id, idx, sentence);
                    match source {
                        model::SentenceSource::Content => content_chunks.push(sentence.to_string()),
                        model::SentenceSource::AnswerText => answer_chunks.push(sentence.to_string()),
                    }
                };
                model::chat_stream(
                    &brain.url,
                    key.as_deref(),
                    &payload,
                    Duration::from_secs(self.config.model_timeout_sec),
                    &mut sink,
                )
            };
            match attempt {
                Ok(sr) => {
                    let usage = sr.reply.usage.clone();
                    rec.chat_end(span, &brain.model, SpanStatus::Ok, &usage, None);
                    let streamed = sr.streamed;
                    let reply = sr.reply;
                    if streamed {
                        let has_tools = !reply.tool_calls.is_empty();
                        let answer_args_ok = reply.tool_calls.iter().any(|c| {
                            c.name == "answer"
                                && serde_json::from_str::<Value>(&c.arguments)
                                    .ok()
                                    .and_then(|v| {
                                        v.get("text")
                                            .and_then(Value::as_str)
                                            .map(|t| !t.trim().is_empty())
                                    })
                                    .unwrap_or(false)
                        });
                        if !has_tools {
                            // Prose answer: the sentences that only
                            // completed at end-of-stream are its last
                            // chunks; journal them in order.
                            for s in &sr.content_tail {
                                let idx = content_chunks.len() + answer_chunks.len();
                                rec.answer_chunk(run_id, idx, s);
                                content_chunks.push(s.clone());
                            }
                            if !content_chunks.is_empty() {
                                rec.answer_stream(
                                    run_id,
                                    "streamed",
                                    content_chunks.len(),
                                    &format!(
                                        "{} streamed its answer in {} sentence chunk(s)",
                                        brain.name,
                                        content_chunks.len()
                                    ),
                                );
                            }
                        } else if answer_args_ok {
                            for s in &sr.answer_tail {
                                let idx = content_chunks.len() + answer_chunks.len();
                                rec.answer_chunk(run_id, idx, s);
                                answer_chunks.push(s.clone());
                            }
                            rec.answer_stream(
                                run_id,
                                "streamed",
                                content_chunks.len() + answer_chunks.len(),
                                &format!(
                                    "{} streamed the answer tool's text in {} sentence chunk(s)",
                                    brain.name,
                                    answer_chunks.len()
                                ),
                            );
                        } else {
                            // A tool round: any content sentences —
                            // including the one the end of the stream
                            // itself proved — were a preamble to the
                            // call, not the answer. Journal them as
                            // chunks (they are speech the brain
                            // produced) under the preamble note, so a
                            // listener speaks them as a preamble and
                            // still speaks the real answer whole.
                            for s in &sr.content_tail {
                                let idx = content_chunks.len() + answer_chunks.len();
                                rec.answer_chunk(run_id, idx, s);
                                content_chunks.push(s.clone());
                            }
                            if !content_chunks.is_empty() || !answer_chunks.is_empty() {
                                let total = content_chunks.len() + answer_chunks.len();
                                rec.answer_stream(
                                    run_id,
                                    "preamble",
                                    total,
                                    &format!(
                                        "{} streamed {total} sentence(s) and then called tools; those chunks were a preamble, not the answer",
                                        brain.name
                                    ),
                                );
                            }
                        }
                    } else if reply.content.is_some() || !reply.tool_calls.is_empty() {
                        rec.answer_stream(
                            run_id,
                            "buffered",
                            0,
                            &format!(
                                "{} answered without a stream; the whole answer is journaled at once",
                                brain.name
                            ),
                        );
                    }
                    let chunks = content_chunks.len() + answer_chunks.len();
                    return Ok(Reply {
                        content: reply.content,
                        tool_calls: reply.tool_calls,
                        brain_name: brain.name.clone(),
                        streamed,
                        chunks,
                    });
                }
                Err(err) => {
                    let reason = err.reason();
                    rec.chat_end(span, &brain.model, SpanStatus::Error, &[], Some(&reason));
                    let spoken = content_chunks.len() + answer_chunks.len();
                    if spoken > 0 {
                        // Speech may already have happened from this
                        // stream. Mixing in another brain's answer
                        // would corrupt the run; abort honestly.
                        rec.answer_stream(
                            run_id,
                            "aborted",
                            spoken,
                            &format!("{} broke mid-stream after {spoken} chunk(s): {reason}", brain.name),
                        );
                        return Err(vec![format!(
                            "{}: the answer stream broke after {spoken} sentence(s) were already journaled: {reason}",
                            brain.name
                        )]);
                    }
                    rec.handoff(&brain.name, &reason, HandoffStatus::Failed);
                    failures.push(format!("{}: {reason}", brain.name));
                }
            }
        }
        Err(failures)
    }
}

/// A model reply tagged with the brain that produced it.
struct Reply {
    content: Option<String>,
    tool_calls: Vec<model::ToolCall>,
    brain_name: String,
    /// CV3: the brain streamed this reply (sentence chunks were
    /// journaled live), and how many chunks were journaled for it.
    streamed: bool,
    chunks: usize,
}

fn provider_of(brain: &Brain) -> &'static str {
    if brain.needs_key {
        "litellm"
    } else {
        "llama.cpp"
    }
}

fn brain_chain_label(brains: &[Brain]) -> String {
    let names: Vec<&str> = brains.iter().map(|b| b.name.as_str()).collect();
    names.join(" then ")
}

/// Echo the model's tool calls back into the conversation, as the API expects.
fn assistant_tool_calls_message(reply: &Reply) -> Value {
    let calls: Vec<Value> = reply
        .tool_calls
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let id = if c.id.is_empty() { format!("call-{}", i + 1) } else { c.id.clone() };
            json!({"id": id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}})
        })
        .collect();
    json!({"role": "assistant", "content": null, "tool_calls": calls})
}
