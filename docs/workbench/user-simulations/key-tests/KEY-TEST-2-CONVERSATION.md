# KEY TEST 2 — Conversation (spec sketch)

Status: SKETCH, adopted 2026-11-03 from the conversational-voice
integration plan (docs/workbench/conversational-voice/INTEGRATION.md).
It becomes a runnable sheet once the phases it exercises exist
(follow-up window CV1, repair CV2, barge-in CV4). Verdict language
follows Key Test 1 discipline: do not upgrade a partial result.
## The test
Style and rigor follow Key Test 1 (same key-tests directory; verdict
language as careful as KT1's P1/P2/P3). Setup: phone on pmOS,
listener running, voice only, wake word allowed **once** at the
start. The operator runs four beats in one sitting:

- **T1 — Task + follow-up.** "Hey phone, open maps and give me
  directions to San Francisco." Then, no wake word: "How long does
  it take to get there?" Pass: both answered from one session; the
  follow-up resolves against the route just opened.
- **T2 — Repair without restart.** Mid-flow, the operator corrects:
  "No — the other San Francisco exit" / or corrects a destination
  the agent misheard. Pass: the agent repairs the one value and
  continues; nothing already established is re-asked.
- **T3 — Barge-in.** The agent begins a long answer; the operator
  interrupts: "Stop — just tell me the time." Pass: playback stops
  within ~0.5 s of the interruption and the new request is answered;
  the interrupted answer never resumes.
- **T4 — Clarification recovery.** A deliberately ambiguous ask
  ("directions to the airport" with two plausible airports in
  context, or a mumbled street name). Pass: exactly ONE targeted
  question, then the task completes on the answer.

Verdicts:
- **P1 — Conversational pass.** All four beats pass in one session:
  one wake word total, no task restarts, no clarification loops,
  median response gap ≤3 s, zero self-interruptions.
- **P2 — Conversational with assistance.** Beats complete, but with
  exactly one assist: one re-wake, one full repeat of a request, or
  one extra clarifying question beyond the first.
- **P3 — Turn-based pass.** All tasks complete but only as discrete
  commands (follow-up window unused or failed; barge-in absent) —
  i.e., Key Test 1 quality, honestly labeled as not yet
  conversation.
- **FAIL**, with codes: **C1** follow-up ignored / treated as new
  session · **C2** repair restarted the task · **C3** barge-in
  ignored, or agent interrupted itself on its own echo · **C4**
  clarification loop (≥3 questions on one value) · **C5** latency
  (median response gap >5 s across the script) · **C6** a bystander
  or TV utterance was accepted as the owner's turn.
