//! Capability broker gates (A1.2 / A1.GATE, INTENT.md safety line).
//!
//! - Deterministic first: a registered block is returned before any
//!   generation is considered.
//! - Predictions never auto-act: they yield AskFirst.
//! - Commit-class actions (send/call/spend/share-location/delete) require
//!   explicit confirmation — including inside generated blocks.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionClass {
    Free,   // read / play / navigate / draft / sandboxed run
    Commit, // send / call / spend / share location / delete
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    RunDeterministic(String),
    Generate,
    AskFirst(String),
    AskToCommit(String),
    RunFree(String),
}

pub struct Broker {
    blocks: HashMap<String, String>, // intent key -> block id
}

impl Broker {
    pub fn new() -> Self {
        Self { blocks: HashMap::new() }
    }

    pub fn register(&mut self, intent_key: &str, block_id: &str) {
        self.blocks.insert(intent_key.to_string(), block_id.to_string());
    }

    /// Deterministic lookup happens before generation, always.
    pub fn dispatch(&self, intent_key: &str) -> Decision {
        match self.blocks.get(intent_key) {
            Some(id) => Decision::RunDeterministic(id.clone()),
            None => Decision::Generate,
        }
    }

    /// A prediction is a question, never an action.
    pub fn anticipate(&self, suggestion: &str) -> Decision {
        Decision::AskFirst(suggestion.to_string())
    }

    /// Gate any action; generated blocks pass through the same gate.
    pub fn gate(&self, class: ActionClass, label: &str, confirmed: bool) -> Decision {
        match class {
            ActionClass::Free => Decision::RunFree(label.to_string()),
            ActionClass::Commit if confirmed => Decision::RunFree(label.to_string()),
            ActionClass::Commit => Decision::AskToCommit(label.to_string()),
        }
    }
}

impl Default for Broker {
    fn default() -> Self {
        Self::new()
    }
}

/// The class each tool declares. Commit is anything that sends, calls,
/// spends, shares location, or deletes user data. Of today's tools only
/// `service_create` qualifies: installing a service commits a (possibly
/// generated) block to the phone, where it will fire on its own timers
/// without anyone asking again — that is the generated-block commit
/// A1.GATE names. `service_stop` is free: svcd's stop keeps the IR and
/// its state under a `stopped` status (it clears timers and grants, it
/// deletes no user data), so stopping is reversible bookkeeping, not a
/// commit. Everything else reads, reports, or only proposes (dry-run).
/// `app_launch` is free as well: it is this header's own navigate class —
/// it displays an app on the phone itself and sends, spends, shares and
/// deletes nothing.
pub fn action_class_for(tool: &str) -> ActionClass {
    match tool {
        "service_create" => ActionClass::Commit,
        _ => ActionClass::Free,
    }
}

/// The exact string a request must carry in `confirm` to authorise this
/// call. For `service_create` it names the service being installed, so
/// confirming one install never authorises a different one. For
/// `app_launch` it names the destination, for the same reason: a
/// confirmation is bound to one destination and spends no other.
pub fn label_for(tool: &str, args: &Value) -> String {
    if tool == "service_create" {
        if let Some(id) = args
            .get("ir")
            .and_then(|ir| ir.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
        {
            return format!("service_create {id}");
        }
    }
    if tool == "app_launch" {
        if let Some(dest) = args
            .get("destination")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            return format!("app_launch {dest}");
        }
    }
    tool.to_string()
}

/// Words of a text, lowercased, apostrophes removed, split on any
/// non-alphanumeric byte. The only normal form the navigation gate
/// compares in.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\'', "")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn joined(text: &str) -> String {
    words(text).join(" ")
}

/// Whether `text` names `destination`: the whole destination phrase
/// appears, or its head word does ("oakland" for "Oakland
/// International", "dentist" for "the dentist"). Head-word matching
/// is what lets a short confirmation ("yes oakland go ahead") bind
/// to the longer proposed name without binding to anything else.
pub fn dest_mentioned(text: &str, destination: &str) -> bool {
    let dest_words = words(destination);
    if dest_words.is_empty() {
        return false;
    }
    if joined(text).contains(&dest_words.join(" ")) {
        return true;
    }
    let stop = ["the", "a", "an", "to"];
    let head = dest_words.iter().find(|w| !stop.contains(&w.as_str()));
    match head {
        Some(h) => words(text).iter().any(|w| w == h),
        None => false,
    }
}

/// Whether the text carries an explicit confirmation marker at all.
/// Token equality, never substring: "work" contains no "ok".
fn is_confirmation_text(text: &str) -> bool {
    let ws = words(text);
    ws.iter().any(|w| matches!(w.as_str(), "yes" | "yeah" | "yep" | "confirm" | "confirmed" | "ok" | "okay" | "sure"))
        || joined(text).contains("go ahead")
        || joined(text).contains("do it")
}

/// Whether this turn explicitly confirms THIS destination: it names
/// the destination alongside a confirmation marker, or it is a bare
/// confirmation (nothing left after filler) for a destination that
/// is already on the table. A turn naming a different place leaves
/// a non-filler word behind and so confirms nothing here.
fn confirms_this_destination(current: &str, destination: &str, on_table: bool) -> bool {
    if !is_confirmation_text(current) {
        return false;
    }
    if dest_mentioned(current, destination) {
        return true;
    }
    if !on_table {
        return false;
    }
    let filler = [
        "yes", "yeah", "yep", "confirm", "confirmed", "go", "ahead", "please", "do", "it", "that", "thats",
        "right", "correct", "ok", "okay", "sure", "thanks", "thank", "you", "now", "just", "is",
    ];
    words(current).iter().all(|w| filler.contains(&w.as_str()))
}

/// A bare routine/departure opening in BRAINFIX v4's sense ("let's
/// go", "leaving now"): the trip began from the owner's patterns,
/// before any destination was named.
fn is_departure_opening(text: &str) -> bool {
    let j = joined(text);
    j.contains("lets go") || j.contains("leaving now")
}

/// Inputs for one navigation-gate decision. History is the session's
/// earlier turns; `established` is the destination a successful
/// launch already set; `wire_confirmed` is the exact wire `confirm`
/// matching this call's destination-bound label.
pub struct NavGate<'a> {
    pub destination: &'a str,
    pub current_user: &'a str,
    pub prior_users: &'a [&'a str],
    pub prior_answers: &'a [&'a str],
    pub established: Option<&'a str>,
    pub wire_confirmed: bool,
}

/// Gate one `app_launch`: navigation stays free ONLY when its
/// destination needs no confirmation or has received its own.
/// `app_launch` remains `ActionClass::Free` (it sends and spends
/// nothing on the phone itself); this gate is the destination-bound
/// confirmation BRAINFIX states as doctrine, enforced where the
/// tool would otherwise run on the brain's say-so alone. The
/// production brain has already been observed launching on an
/// unconfirmed inference with the instruction verbatim in its
/// prompt, so the prompt alone is not the enforcement.
pub fn navigation_gate(gate: NavGate<'_>) -> Decision {
    let label = format!("app_launch {}", gate.destination.trim());
    if gate.wire_confirmed {
        return Decision::RunFree(label);
    }
    if let Some(done) = gate.established {
        if words(done) == words(gate.destination) || dest_mentioned(done, gate.destination) {
            return Decision::RunFree(label);
        }
    }
    let prior_proposed = gate.prior_answers.iter().any(|a| dest_mentioned(a, gate.destination));
    let prior_named = gate.prior_users.iter().any(|u| dest_mentioned(u, gate.destination));
    let on_table = prior_proposed || prior_named;
    if on_table && confirms_this_destination(gate.current_user, gate.destination, on_table) {
        return Decision::RunFree(label);
    }
    let named_now = dest_mentioned(gate.current_user, gate.destination);
    let opening = gate.prior_users.first().copied().unwrap_or(gate.current_user);
    let departure = is_departure_opening(opening) && !dest_mentioned(opening, gate.destination);
    if named_now && !departure && !prior_proposed {
        return Decision::RunFree(label);
    }
    Decision::AskToCommit(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_block_preferred_over_generation() {
        let mut b = Broker::new();
        b.register("navigate_tabby", "block-nav-v1");
        assert_eq!(b.dispatch("navigate_tabby"), Decision::RunDeterministic("block-nav-v1".into()));
        assert_eq!(b.dispatch("unknown_thing"), Decision::Generate);
    }

    #[test]
    fn prediction_asks_first() {
        let b = Broker::new();
        assert_eq!(b.anticipate("Going to Tabby's?"), Decision::AskFirst("Going to Tabby's?".into()));
    }

    #[test]
    fn navigation_labels_and_mentions_bind_to_one_destination() {
        let args = serde_json::json!({"app": "maps", "destination": "SFO"});
        assert_eq!(label_for("app_launch", &args), "app_launch SFO");
        assert!(dest_mentioned("yes oakland go ahead", "Oakland International"));
        assert!(dest_mentioned("The farmers market - head there?", "farmers market"));
        assert!(!dest_mentioned("yes go ahead", "work"));
        assert!(!dest_mentioned("no navigate to the airport first", "work"));
    }

    #[test]
    fn navigation_gate_spends_only_its_own_confirmation() {
        let ask = |d: &str| Decision::AskToCommit(format!("app_launch {d}"));
        let run = |d: &str| Decision::RunFree(format!("app_launch {d}"));
        // An inference with nothing on the table waits.
        assert_eq!(
            navigation_gate(NavGate { destination: "SFO", current_user: "i need to get to my flight", prior_users: &[], prior_answers: &[], established: None, wire_confirmed: false }),
            ask("SFO")
        );
        // Its own confirmation, after its own proposal, runs.
        assert_eq!(
            navigation_gate(NavGate { destination: "SFO", current_user: "yes sfo go ahead", prior_users: &["i need to get to my flight"], prior_answers: &["SFO - head there?"], established: None, wire_confirmed: false }),
            run("SFO")
        );
        // A confirmation naming a different place confirms nothing here.
        assert_eq!(
            navigation_gate(NavGate { destination: "work", current_user: "yes airport go ahead", prior_users: &["lets go"], prior_answers: &["Work - head there?"], established: None, wire_confirmed: false }),
            ask("work")
        );
        // Naming inside a departure flow waits for the read-back yes.
        assert_eq!(
            navigation_gate(NavGate { destination: "farmers market", current_user: "the farmers market", prior_users: &["lets go"], prior_answers: &["where are we headed today?"], established: None, wire_confirmed: false }),
            ask("farmers market")
        );
        assert_eq!(
            navigation_gate(NavGate { destination: "farmers market", current_user: "yes go ahead", prior_users: &["lets go", "the farmers market"], prior_answers: &["where are we headed today?", "The farmers market - head there?"], established: None, wire_confirmed: false }),
            run("farmers market")
        );
        // A user-named destination outside a departure flow runs at once.
        assert_eq!(
            navigation_gate(NavGate { destination: "San Francisco", current_user: "directions to San Francisco", prior_users: &[], prior_answers: &[], established: None, wire_confirmed: false }),
            run("San Francisco")
        );
    }

    #[test]
    fn commit_requires_confirmation_even_for_generated() {
        let b = Broker::new();
        assert_eq!(b.gate(ActionClass::Commit, "send text", false), Decision::AskToCommit("send text".into()));
        assert_eq!(b.gate(ActionClass::Commit, "send text", true), Decision::RunFree("send text".into()));
        assert_eq!(b.gate(ActionClass::Free, "play video", false), Decision::RunFree("play video".into()));
    }
}
