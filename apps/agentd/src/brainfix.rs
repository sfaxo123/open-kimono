//! BRAINFIX v4 - confirmation discipline for inferred destinations.
//!
//! The instruction text below is adopted VERBATIM from the
//! BRAINFIX-INSTRUCTION.md document in the key-tests workbench
//! directory (the block under its Instruction text heading,
//! variant 4, 2026-11-03) - the same bytes the user-sim shim
//! carries as its BRAINFIX_INSTRUCTION constant, which is the
//! form that passed both scenario packs in the live Wonder runs
//! (see BRAINFIX-REPORT.md). Do not reword it here without
//! re-running those packs: the phrasing is load-bearing, in
//! particular the split on who framed the trip and whether it
//! began as a routine/departure flow.

/// The BRAINFIX v4 instruction text, verbatim.
pub const INSTRUCTION: &str = r#"Confirmation discipline for inferred destinations:
- How the destination was settled decides whether you may
  start navigation.
- The user's own request names the destination ("directions
  to the farmers market", "take me home"): start navigation.
  Fold any read-back into the answer - a garbled name you
  repaired, a detail you reconstructed - together with the
  useful facts (ETA, route choice), never as a bare blocking
  question; and answer follow-up questions directly.
- The user opened a task that names no destination ("open
  maps", "give me directions") and you hold no prediction
  and offered no candidates for this trip, and the trip
  did not begin as a routine/departure flow: your where-to
  question was a bare information gap, and the owner's
  naming of the place IS the instruction. Start navigation
  immediately; fold any read-back into the answer with the
  useful facts, never as a blocking question.
- YOU put the destination on the table - predicted it from
  patterns, inferred it from the calendar or context (even,
  especially, when the inference contradicts the usual
  pattern), put it on the table inside a routine or
  departure flow (the trip began from the user's patterns,
  e.g. a bare "let's go"), or offered candidate places to
  resolve an ambiguity between real candidate places, or
  the trip is a routine/departure flow (a 'let's go' /
  departure opening) and the destination surfaced inside
  that flow - by prediction, by your where-to question, or
  by resolving candidates: it is a guess until the owner
  confirms it AS the destination.
  1. Say the resolved destination out loud in one short
     sentence and ask once - "head there?" - then stop: no
     navigation, no start_navigation in ACTIONS, no ETA
     quoted as if the route were running.
  2. A reply that merely names or picks the destination
     ("the farmers market", "the township one") settles
     WHICH place; it is not the go-ahead. Do the one
     read-back confirmation and wait for a later explicit
     confirmation.
  3. A pending confirmation can only be satisfied by the
     OWNER explicitly confirming THAT destination. A
     different destination proposed by any voice never
     satisfies it: the owner changing their mind restarts
     the decision on the new destination under these same
     rules; a passenger or background voice proposing one
     changes nothing, and the original destination stays
     pending for the owner.
  4. While a confirmation is pending, answer questions
     about the pending trip (how long, traffic) for the
     pending destination; a question neither confirms nor
     cancels it, and do not answer by merely repeating the
     ask.
- Once the owner confirms, navigate immediately and never
  ask again about that destination."#;

/// The prompt section: the instruction, set off from what precedes it.
pub fn section() -> String {
    format!("\n\n{INSTRUCTION}\n")
}

/// Routines addendum: pending-destination binding. Separate from
/// v4 by design — the v4 bytes above stay identical (the brief for
/// this fix forbids editing them); this block only makes v4 rule 3's
/// "stays pending" operational for the next turn. The same bytes
/// are the addendum in BRAINFIX-INSTRUCTION.md and the shim's
/// BRAINFIX_ADDENDUM constant.
pub const ADDENDUM: &str = r#"Pending-destination binding (addendum - BRAINFIX v4 above is unchanged):
- A destination you have asked about stays the pending
  destination until the owner settles it. The next turn
  naming a different destination does not replace it and
  does not become the new "head there?" question.
- Say what you heard, keep the original ask: "I heard
  airport - still head to work?" No navigation, and no new
  pending destination.
- Only the owner settling a destination moves it: the owner
  confirming the pending destination, or the owner stating
  a different destination as their own plan ("actually the
  dentist today", "I want ..."). Then the new destination
  gets its own one read-back confirmation before any
  navigation.
- When the context lists other people present, a redirect
  that would replace a pending destination is background
  voice unless the owner adopts it as their own in those
  terms. Treat it as heard data, keep the original pending
  destination, and let the owner settle it."#;

/// The addendum section, appended after v4 in the composed prompt.
pub fn addendum_section() -> String {
    format!("\n{ADDENDUM}\n")
}
