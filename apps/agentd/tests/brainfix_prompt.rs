//! BRAINFIX v4 in the real prompt: the composed system prompt must carry
//! the confirmation-discipline instruction verbatim, on top of (never
//! instead of) the base instructions. If a refactor drops or rewords
//! the section, this test is the tripwire.

use std::fs;

use agentd::agent::Agent;
use agentd::brainfix;
use agentd::config;
use agentd::journal::Journal;
use agentd::phone::ProbeRoot;

fn agent() -> Agent {
    let dir = std::env::temp_dir().join(format!("agentd-brainfix-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("fixture dir");
    let cfg = "listen = \"127.0.0.1:8797\"\ngateway_url = \"http://127.0.0.1:1/v1/chat/completions\"\ngateway_model = \"fake\"\nkey_file = \"/dev/null\"\n";
    Agent::with_root(config::load(cfg).expect("a valid config"), Journal::new(64), ProbeRoot(dir))
}

#[test]
fn the_prompt_carries_brainfix_verbatim() {
    let text = agent().system_prompt();
    assert!(
        text.contains(brainfix::INSTRUCTION),
        "the composed prompt must contain the BRAINFIX instruction byte for byte"
    );
}

#[test]
fn the_prompt_carries_the_flow_based_split() {
    // The key clauses of v4, each on one line of the source text: the
    // settling rule, the user-framed branch (incl. the v4 flow clause),
    // the agent-framed branch, the naming-is-not-confirmation rule, the
    // owner-only pending-confirmation rule, and the confirm-once rule.
    let text = agent().system_prompt();
    for needle in [
        "Confirmation discipline for inferred destinations:",
        "How the destination was settled decides whether you may",
        "did not begin as a routine/departure flow",
        "YOU put the destination on the table",
        "WHICH place; it is not the go-ahead",
        "A pending confirmation can only be satisfied by the",
        "Once the owner confirms, navigate immediately and never",
        "head there?",
    ] {
        assert!(text.contains(needle), "the prompt is missing the BRAINFIX clause {needle:?}");
    }
}

#[test]
fn the_routines_addendum_follows_v4_without_touching_its_bytes() {
    // The fix for the routines pack appends a binding addendum AFTER
    // v4; v4 itself stays byte-identical (2803 bytes, as landed).
    assert_eq!(brainfix::INSTRUCTION.len(), 2803, "BRAINFIX v4 bytes changed");
    let text = agent().system_prompt();
    assert!(text.contains(brainfix::ADDENDUM), "the composed prompt must carry the addendum");
    let v4_at = text.find(brainfix::INSTRUCTION).expect("v4 present");
    let add_at = text.find(brainfix::ADDENDUM).expect("addendum present");
    assert!(v4_at < add_at, "the addendum follows v4, never interleaves it");
    for needle in ["stays the pending", "does not replace it", "still head to work?"] {
        assert!(text.contains(needle), "the prompt is missing the addendum clause {needle:?}");
    }
}

#[test]
fn brainfix_composes_with_the_base_prompt_rather_than_replacing_it() {
    let text = agent().system_prompt();
    for needle in ["You are agentd", "Acting is a tool call, never a promise", "SERVICE IR"] {
        assert!(text.contains(needle), "the base prompt lost {needle:?}");
    }
    let base_at = text.find("You are agentd").expect("base present");
    let fix_at = text.find("Confirmation discipline").expect("brainfix present");
    assert!(base_at < fix_at, "BRAINFIX is appended to the base prompt, not a replacement");
}
