# User simulations — how a human talks to the agentic phone

A black-box acceptance corpus for the agentic phone: scripted
simulated users interacting by **voice** and **text**, with the
behaviour a correct agent is expected to show at each turn. It
tests the *interaction contract* — prediction, confirmation,
correction, refusal — not any one implementation. It was
written against the project's end state: a fully agentic
phone, intelligently managed, where everything running has a
purpose for the user and nothing acts on the user's behalf
without the confirmation the situation calls for.

This corpus is Track-B-side test material. It does not
describe, and must not be read as, the design of any
implementation track.

## Layout

- `corpus/core.jsonl` — the core set: 48 scenarios, one JSON
  object per line, IDs `SIM-001` … `SIM-048`.

## Field guide

Per scenario:

| field | meaning |
|---|---|
| `id` | stable ID, `SIM-NNN` |
| `category` | one of the coverage categories below |
| `channel` | `"voice"` or `"text"` — how the user is talking |
| `persona` | short role note (e.g. "weekday commuter") |
| `context` | object: time, weekday, location, connectivity, `battery_pct`, and scenario facts a real agent could legitimately know (`prior_patterns`, `calendar_note`, `active_route`, `alert`, …). `trigger: "bluetooth_connected_car"` marks the car scenarios |
| `turns` | ordered user turns; each turn is `{user, expected}` |
| `expected` | scenario-level expectation — mirrors the final turn's `expected` |

Per turn, `expected` is a compact object:

| field | meaning |
|---|---|
| `intent` | the intent the agent should resolve this turn to |
| `actions` | ordered action tokens the agent should take (snake_case, implementation-agnostic) |
| `must_confirm` | `true` when the agent must get an explicit yes *before* the consequential action (sending, paying, navigating, restarting, deleting, sharing) |
| `must_not` | action tokens that would fail the turn (silent sends, double-booking, acting on a garbled value, …) |
| `success` | one human-readable sentence: what a pass looks like |

Voice turns that carry realistic speech-recognition noise are
flagged `"asr_noise": true` on the turn (19 turns): homophones
("hole foods" for Whole Foods, "john with an aitch"),
missing punctuation, run-ons, and spelled-out corrections
("fifteen one five"). The noise is in the `user` text itself —
a runner should pass it verbatim.

## How a runner consumes it

1. Read `core.jsonl` line by line; each line is one scenario.
2. Stand up the agent under test with only the scenario's
   `context` as its world (contacts, calendar, patterns,
   homelab state are all stated there — nothing else exists).
3. For each turn, feed `turns[i].user` on the scenario's
   `channel` (voice turns as the literal noisy transcript —
   or synthesised audio of it, if testing a speech pipeline).
4. Score the turn against its `expected`:
   - resolved intent matches `intent`;
   - every `actions` token happened (in order where order
     matters, e.g. read-back *before* send);
   - if `must_confirm` is `true`, the consequential action
     did **not** happen before the user's explicit yes;
   - no `must_not` token happened;
   - the outcome is consistent with `success`.
5. A scenario passes only if every turn passes. Report per
   category so a failure cluster (e.g. all confirmations)
   is visible.

Agent-initiated behaviour (the car case: the phone speaks
first when Bluetooth connects) is represented by the
`trigger` in `context`; the first user turn is the human's
reply to that prompt.

## Coverage

| category | scenarios | voice | text | what it pins |
|---|---|---|---|---|
| car-bluetooth-destination | 6 | 5 | 1 | the defining case: predict from patterns, ask, confirm before navigating — routine, weekend-differs, passenger present, user corrects, ASR-garbled, text variant |
| reminders-alarms-timers | 5 | 2 | 3 | set/extend/snooze/cancel; ambiguous AM/PM must be clarified |
| messaging-calls | 6 | 4 | 2 | dictation with read-back before send, name correction, ambiguous contact, hands-free call, incoming read + reply, mid-message change |
| calendar | 5 | 1 | 4 | create/move without duplicating, conflict flagged not double-booked, leave-by time with traffic, recurring series |
| navigation-places | 5 | 2 | 3 | hours with closing times, on-the-way stop with detour cost stated, racing a closing time warns first, parking, closure-aware routing |
| homelab | 5 | 2 | 3 | status questions answered for the right machine, storage, ambiguous target clarified, restart only after explicit confirmation |
| media | 4 | 2 | 2 | play/pause/volume/queue/resume-at-position |
| quick-facts | 4 | 2 | 2 | conversions and arithmetic with the exact values from the request |
| multi-turn-corrections | 4 | 3 | 1 | "no, the other one", mid-sentence edits, a misheard number caught by stating it back, mid-route destination change |
| privacy-consent | 4 | 1 | 3 | money and location sharing confirmed and scoped, bulk delete listed before it happens, another person's messages refused |
| **total** | **48** | **24** | **24** | 62 user turns, 19 with ASR noise |

## Honest limits

- These are **simulated humans, written by an agent** — not
  field data. Real users are ruder, vaguer, and change their
  minds mid-word more often than these scripts do.
- Expected values that depend on the world (ETAs, closing
  times, conversion results) are stated in `context` or the
  `success` line so a runner can check arithmetic, but they
  are illustrative, not measured.
- The corpus pins *observable behaviour*, not wording: an
  agent that says the right thing in different words passes;
  the `success` line, not a transcript, is the standard.
- Confirmation expectations encode this project's standing
  rule (consequential actions wait for an explicit yes).
  Another product could legitimately choose otherwise; the
  corpus would need its `must_confirm` fields revisited.
- No real people, addresses, or account data appear anywhere
  in the corpus; names and places are generic stand-ins.
