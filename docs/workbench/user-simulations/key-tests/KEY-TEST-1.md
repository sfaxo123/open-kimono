# KEY TEST 1 - a conversation with the phone: wake word, maps, directions

Operator order 2026-11-03 (~07:34 PT), the owner, verbatim test:
"have a conversation with phone - it can use a wake word -
I want something simple like open maps and give me directions".

This is the first key test for the agentic phone: not a
benchmark suite, one ordinary human interaction done end to
end. If the phone cannot do this, nothing fancier matters.

## The flow under test

1. WAKE. The owner wakes the phone with a wake phrase
   ("hey phone"). Voice scenarios in the pack open with it.
2. REQUEST. The owner asks for maps / directions in natural
   speech, possibly garbled by speech recognition.
3. RESOLVE. The agent works out the destination:
   - stated outright -> use it;
   - implied by context (calendar, prior patterns) -> it is
     a GUESS: say the guess out loud and confirm it (step 4);
   - ambiguous (two places fit) -> CLARIFY once, never guess;
   - garbled (ASR-mangled name) -> resolve against contacts /
     context and READ BACK the name it resolved to.
4. CONFIRM - at most ONE confirmation, and only when the
   scenario marks it: the destination was inferred or had to
   be clarified. An explicit, unambiguous destination needs
   no confirmation - asking anyway is the over-confirmation
   failure the undo pack exists to catch. A stop/undo never
   waits for confirmation.
5. ACT. Maps opens on the destination AND the agent speaks a
   short directions summary (route + rough time). Opening
   maps silently, or talking without opening, are both
   half-failures.
6. CONVERSE. Follow-ups stay in context: "how long will it
   take", "avoid tolls", "reroute", a change of mind, a
   stop. The destination and route history are not lost
   between turns.

## The action contract

Side effects are reported as snake_case action tokens. The
two headline actions of this test:

- open_maps(destination) - maps opens on the destination
  (reported token: open_maps);
- give_directions(summary) - the spoken directions summary
  (reported token: give_directions).

Supporting tokens the scenarios expect to see named in
replies/actions: start_navigation, report_eta,
report_traffic, reroute / reroute_avoid_tolls,
cancel_navigation, ask_destination (the asking, not acting).

Standing rules that bind every scenario:

- Background voices are never obeyed. A passenger (or a TV)
  naming a different destination changes nothing; the agent
  arbitrates with the OWNER, briefly, and the owner decides.
- No navigation before the single confirmation when the
  scenario marks must_confirm - an action reported before
  the agent has asked and the owner has confirmed fails the
  scenario, however good the guess was.
- An undo / stop acts immediately, in the same turn, with
  no confirmation question.
- Corrections are acknowledged and applied; the corrected
  destination replaces the old one everywhere (ETA answers
  after a change of mind are for the NEW destination).

## Pass / fail criteria

Mechanical (scored by tools/user-sim over keytests.jsonl):

- intent: the scenario intent, or any one intent_alias,
  matched in the combined replies + actions (keyword rules
  and synonym groups per the harness README; aliases name
  the demonstrated behaviour in plain words - the rescore
  round proved compound labels fail correct behaviour).
- must_confirm: no action before ask + confirmation in the
  marked scenarios (see flow step 4).
- must_not: none of the listed strings in any action.
- correction: a correction turn is never met with the
  previous reply repeated verbatim.
- The mock must score 100% (plumbing proof) and the pack
  must lint clean before any live run counts.

Judged by a human reading the live transcripts (KEY-TEST-1-
RESULTS.md): the spoken summary was actually spoken (route
and time, not just "done"); a garbled name was read back,
not silently acted on; a clarification was ONE question,
not an interrogation; the agent sounded like a phone
agent, not a form.

## What this test does NOT score - honestly

- Real wake-word detection on hardware. The harness feeds
  text; the wake phrase is framing, not a detection task.
  Tonight live run on the phone covers wake-word reality.
- Real microphone ASR. Voice turns pass the harness
  deterministic ASR-noise transform (homophones, number
  flips, name garbling, dropped words); that grades the
  brain read-back defences, not a recogniser.
- A real maps application. open_maps is a reported action
  token at the conversation level; whether the phone
  build opens a real maps app is a Track A / device
  question, out of scope for this pack.

So a PASS here means: the BRAIN passes the owner key test at
the conversation level. The device proof is separate and
is not claimed by this pack.

## Scenario map (keytests.jsonl)

| id | shape | what it proves |
|---|---|---|
| KT-001 | voice, 2 turns | happy path: wake + explicit airport, maps + summary, ETA follow-up |
| KT-002 | voice, 3 turns | pattern-predicted work: guess is confirmed once, then toll-free reroute |
| KT-003 | voice, 2 turns | explicit home; ASR -teen/-ty flip in the follow-up does not corrupt the ETA |
| KT-004 | voice, 3 turns | two Springfields: clarify, never guess; confirm the resolved place once |
| KT-005 | voice, 3 turns | background voice names the airport; owner office route stands |
| KT-006 | voice, 2 turns | "open maps" with no destination: ask once, then act |
| KT-007 | text, 3 turns | text happy path + ETA + avoid tolls, context held |
| KT-008 | voice, 3 turns | change of mind gym -> library; ETA answers follow the new destination |
| KT-009 | voice, 2 turns | stop navigation: immediate, no confirmation |
| KT-010 | voice, 2 turns | ASR homophone (whole -> hole foods) still resolves to the real place |
| KT-011 | voice, 1 turn | ASR name garble (Elena -> Alana) resolved by read-back, no invented contact |
| KT-012 | voice, 3 turns | flight in calendar: airport inferred, confirmed once, traffic follow-up |
| KT-013 | voice, 3 turns | accident ahead: reroute + new ETA, conversation continues |
| KT-014 | text, 3 turns | multi-stop build: coffee -> office -> home, stops added by correction |
| KT-015 | voice, 3 turns | passenger asks for a mall stop; agent checks with the owner, route unchanged |
| KT-016 | voice, 1 turn | one-shot: market + avoid tolls + ETA in a single summary (ASR week -> weak) |

Pattern scenarios (KT-002 work, KT-003 home, KT-012
airport) state their learning boundaries in the scenario
context, journeys-pack style: what one trip may teach
(may_learn) and what it must never teach (must_not_infer).
A single airport run must not become "Wednesdays are
airport days"; a pattern strengthens only by repetition.
