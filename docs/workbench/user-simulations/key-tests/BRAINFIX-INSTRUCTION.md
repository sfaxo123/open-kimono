# BRAINFIX — confirmation discipline for inferred destinations

KEY TEST 1 follow-up (2026-11-03). A standalone instruction
module for the phone agent's system prompt: the block under
"Instruction text" is written to be adopted verbatim (it was
tested verbatim, wired into the user-sim shim's system prompt
on top of the existing rules). It fixes one defect in all its
costumes: the brain treating anything that resolves a
destination — its own inference, a naming that answers its
own question, or the wrong speaker entirely — as the
confirmation to navigate (KT-012, KT-004, RT-002, RT-004,
RT-010; pilot control EDGE-013 is the same failure family).

This is variant 4. Variant 1 fixed KT-012/KT-004 but
over-blocked destinations the user had named outright
(KT-010, KT-016); variant 2's "whose move settled the
destination" distinction corrected that, but its "or asked
where to go" clause lumped a bare information gap into the
agent-framed bucket and flipped KT-006 PASS -> FAIL. Root
adjudicated the KT-006 vs RT-002 spec conflict; both packs
stand, under one distinction - WHO FRAMED THE TRIP.
Agent framed it (the agent predicted a destination, a
routine/departure flow is running, or the agent offered
candidate places to resolve an ambiguity): a naming/pick
settles WHICH place; one read-back confirmation is still
required before navigating. User framed it (the user
opened a task with no destination - "open maps" - and no
agent prediction or candidates are in play; the agent's
where-to question is a bare information gap): the owner's
naming IS the instruction - act immediately, fold any
read-back into the answer, never as a blocking question.
Variant 3 splits the "or asked where to go" clause
accordingly. Variant 4 keys the split on how the trip
OPENED: variant 3's user-framed bullet keyed on "you hold
no prediction and offered no candidates" - literally true
when the agent asks a bare "where to today?" INSIDE a
departure flow, so RT-002's naming took the user-framed
branch and navigated unconfirmed (RT-004, same opening
with a prediction voiced, passed). Variant 4: the
user-framed bullet gains "and the trip did not begin as a
routine/departure flow"; the agent-framed bullet gains
"or the trip is a routine/departure flow (a 'let's go' /
departure opening) and the destination surfaced inside
that flow - by prediction, by your where-to question, or
by resolving candidates". Nothing else changed. Variant
history and scores: BRAINFIX-REPORT.md.

## Instruction text (verbatim)

Confirmation discipline for inferred destinations:
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
  ask again about that destination.

---

## Addendum (CV2): the transcription-confidence axis

BRAINFIX above governs **provenance**: whether a value the AGENT
inferred may act without the owner's read-back. It says nothing
about the failure one step earlier — the user stated the value
plainly, and the speech recognizer heard a different word. CV2
adds that axis, in agentd, as three strata over the confidence
the transcription carries. The axes are independent and compose:

- **Provenance (BRAINFIX)** decides whether an inferred value may
  act. It is unchanged: every rule above applies verbatim, to
  every turn, whatever stratum the turn fell in.
- **Confidence (this addendum)** decides how a possibly-misheard
  value is handled before provenance is even a question.

### The signal, named honestly

The confidence on the wire today (`asr_confidence` on
`POST /request`) is voiced's **acoustic-level proxy**: the whole
utterance's RMS level mapped to 0.8 (clearly audible) or 0.4
(quiet). It is NOT a decoder confidence — the Moonshine driver
contract exposes no per-word or per-utterance score, and no finer
signal exists on the live path. The strata are written against
whatever value the wire carries; a finer signal later changes
only the thresholds. A request carrying NO confidence behaves
exactly as agentd did before CV2 — absence of signal is never
treated as doubt.

### The strata

Thresholds are config (`repair_high`, default 0.7; `repair_low`,
default 0.55), and every classified turn is journaled as a
`repair_stratum` event with its stratum, confidence, reason, and
span.

- **HIGH** (confidence ≥ `repair_high`) — act exactly as today.
- **MEDIUM** (between the thresholds) — proceed with the request,
  and restate the heard span naturally in the spoken answer while
  doing so, so the user can catch a mishearing. The instruction
  to restate reaches the brain as a user-role background note —
  data, in the same shape as CV1's entity summary — never as
  system prompt. The system prompt, BRAINFIX included, is
  byte-identical in every stratum.
- **LOW** (confidence < `repair_low`) — do not run the brain, do
  not call a tool, do not reach the broker. Ask exactly ONE
  targeted question about the uncertain span (the destination of
  a directions request, the app of an open request, otherwise the
  request as heard), keep the session's slots and the original
  request, and resume the original task from the spoken answer:
  the composed request — original words with the span replaced by
  the answer — is what the brain serves, journaled verbatim as
  `repair_resumed`. The answer is accepted as spoken even if it
  arrives at low confidence too: a repair never asks twice.

### Composition rules (the part that must not drift)

1. A LOW turn cannot weaken a BRAINFIX must_confirm case: it
   cannot act at all. If the resumed request contains an
   agent-inferred value, BRAINFIX adjudicates it exactly as it
   would a fresh request — the confirmation is still owed.
2. A MEDIUM restatement is not a confirmation and does not
   substitute for one. If BRAINFIX or the broker requires a
   confirmation for a turn, the turn still asks; the restated
   span rides along in the answer, nothing more.
3. A repair answer is the owner's words, not a confirmation
   token: resuming composes them into the request and the
   composed turn then passes the broker gate and BRAINFIX
   adjudication fresh, like any other turn. Repair is never a
   confirmation channel.
4. Composition never launders provenance. The composed request
   contains only what the user said across the two turns — the
   original transcription and the spoken answer. If the agent
   inferred the value, no repair wording may present it as
   user-stated; the `repair_resumed` journal record shows the
   original, the heard span, the answer, and the composition, so
   the audit never has to take the composition on faith.

## Addendum (routines): pending-destination binding

BRAINFIX v4 rule 3 already states the doctrine — a different
destination proposed by any voice never satisfies a pending
confirmation, and the original destination stays pending. The
routines pack showed the next-turn failure v4's wording left
open: with work pending ("head there?"), a passenger's
"navigate to the airport first" was re-asked as the new pending
question ("airport first — head there?"). No navigation ran,
but the pending slot had been replaced by the wrong speaker,
and only the owner's explicit correction restored it. The v4
text stays byte-identical; the block below is appended after
it in both composed prompts (agentd `brainfix::ADDENDUM`, shim
`BRAINFIX_ADDENDUM`) and nowhere replaces it. Agentd also
enforces the same binding deterministically at the tool gate
(`broker::navigation_gate`); this addendum is the brain-facing
half, the gate is the backstop when wording alone is ignored.

### Instruction text (addendum, verbatim)

Pending-destination binding (addendum - BRAINFIX v4 above is unchanged):
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
  destination, and let the owner settle it.
