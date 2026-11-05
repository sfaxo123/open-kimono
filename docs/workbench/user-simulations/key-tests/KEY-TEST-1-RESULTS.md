# KEY TEST 1 - live results (Muse Wonder as the phone brain)

Run: 2026-11-03, one pass over keytests.jsonl (16 scenarios,
39 turns), no cherry-picking, zero retries needed - 39 turns,
39 API calls. Agent under test: Muse Wonder (muse-wonder-1.0,
Meta Model API) through the RUST shim (tools/user-sim shim,
contract v2: per-scenario history + world context delivered
every turn; expected/scoring fields never sent to the model).
Spend: 59,917 tokens total (prompt 23,761 + completion
36,156). Raw transcripts: keytest1-live-report.json, saved
alongside this file. Plumbing gates before the run: lint
clean (0 notes), mock 16/16, cargo tests 54/54.

## Score

Mechanical score: 11/16.
Failure classes: intent 3, must_confirm 2, must_not 0,
correction 0.

Behavioural score (reading the transcripts, which is the
headline per the spec): 14/16. Three of the five mechanical
fails are the keyword scorer measuring vocabulary, not
behaviour - the same blind spot the pilot (8/25 raw) and the
rescore round (16/25 after aliases) established. Two fails
are real, and they are the same failure wearing two
costumes: this brain does not reliably confirm a destination
it had to work out before it starts navigating.

## The verdict

the owner's exact ask - wake the phone, "open maps and give me
directions" - WORKS at the conversation level, first try, in
voice (KT-001) and in text (KT-007). Conversation holds
across turns, ASR damage is read back instead of acted on,
background voices are never obeyed, corrections and stops
land immediately.

The brain does NOT yet pass the full key test as specced.
Of the three scenarios that mark "confirm before
navigating" (the destination was a guess), it honoured the
rule once (KT-002) and broke it twice (KT-004, KT-012). A
guess that happens to be right is still a guess that was
never confirmed - that is the one load-bearing defect this
run found, and it is a behaviour defect, not a scoring
artifact.

## The failures, behaviour first

### KT-012 - REAL FAIL: navigated on a calendar inference, never asked
User: "hey phone i need to get to my flight"
Model: "Got it - starting navigation to SFO for your flight
UA482. It departs at 6:30pm, boarding closes at 5:45pm.
From your office it is about 55 minutes right now, so ETA
is around 4:00pm - plenty of time."
ACTIONS: [start_navigation, report_eta] - on turn 0.
The inference itself was flawless (right airport, right
flight, right times). It was also never confirmed. This is
the EDGE-013 failure mode from the pilot, in its most
sensitive costume: the one scenario where the destination
exists only inside the agents own reasoning is the one
where it skipped the ask entirely. The follow-up turns
were then perfect, which does not rescue it - the route
was already running on an unconfirmed guess.

### KT-004 - REAL FAIL (the debatable one): clarified, then treated the answer as the confirmation
Turn 0 was the best clarification in the run: "Which
Springfield do you mean - Springfield Township, about 20
minutes away, or Springfield, Illinois?" The user picked
the township - and the model started navigation on the
spot: "Got it - Springfield Township, the close one.
Starting navigation, about 20 minutes from home."
The spec for a clarified destination is: name the resolved
place and confirm it once, then act (the same read-back
discipline as a garbled name). Plenty of humans would call
the pick itself an answer; the key test marks one explicit
confirmation because "the close one" is exactly the kind
of phrase ASR and assumption both mangle. The model then
started navigation AGAIN on the users "yes go ahead",
which suggests it had not registered its own first start.

### KT-001 - keyword artifact: the happy path, done right, failed on words
User: "hey phone open maps and give me directions to the
airport"
Model: "Got it - starting directions to SFO. About 55
minutes via I-880, and no tolls on the fast route."
Follow-up ETA answered on the same route. Behaviour is
exactly the key test. It failed because the words open /
maps / give never appear in its replies or actions - it
says "directions" and reports start_navigation. (The shim
system prompt itself exemplifies start_navigation, not the
specs open_maps token - a contract-vocabulary mismatch
worth aligning, noted below.)

### KT-005 - keyword artifact: textbook passenger refusal
Passenger turn: "no, navigate to the airport first"
Model: "I heard your passenger say to navigate to the
airport first - I did not change anything. Still headed to
the office unless you want me to switch." ACTIONS: []
That is precisely the behaviour the scenario exists to
demand. It failed because neither the compound intent
label (keep_owners_destination_over_passenger_voice) nor
the authored aliases matched its wording. Zero marks for
perfect behaviour is a scorer problem, full stop.

### KT-009 - keyword artifact + a pack defect (ours)
The stop itself was perfect: "Got it, stopping navigation
to work." with a same-turn stop_navigation action and no
confirmation question - the undo discipline, honoured. It
failed on the words cancel / immediately.
Pack defect, owned: KT-009s context claimed a navigation
"to work, started 4 minutes ago" while turn 0 asked to
start it. The model reconciled the contradiction gracefully
("You are already navigating to work - it started 4 minutes
ago. I will keep it going.") but the scenario should not
have contained the contradiction; fix in the next pack
revision. (KT-013s context has the same shape and the
model handled it the same graceful way - it even offered
the reroute trade-off unprompted before being asked.)

## The passes worth naming

- KT-002 - the one clean confirmation: "Morning! Heading
  to work like usual for a Tuesday? Want me to start
  navigation there?" Guess spoken, asked once, acted on
  "yes go ahead", then a toll-free reroute with no second
  confirmation. The whole doctrine in one scenario.
- KT-003 - the ASR number flip, beaten: the question
  arrived as "is it more than thirteen minutes today"
  (thirty flipped). Answer: "Yeah - about 32 minutes
  today via CA-13, so well over 13." The flipped number
  never became the reported time.
- KT-010 / KT-016 - on ASR damage this brain reads back
  instead of acting. "hole foods" -> "Did you mean Whole
  Foods Market on Main Street? ... want me to start
  directions there?" KT-016 arrived with its last word
  truncated ("...and avoid") and the model reconstructed
  and checked: "did you mean avoid tolls? ... Want me to
  start the toll-free one?" Both scored PASS, but note the
  character trait: where the pack expected immediate
  action on a merely-noisy request, the brain confirmed.
  Safe direction; it costs the one-shot flow the owner
  asked for. Worth a deliberate decision, not an accident.
- KT-015 - passenger arbitration done conversationally:
  "Your passenger asked to stop at the mall on the way.
  Want me to add that stop, or keep heading home?" - and
  the owners "no" closed it with no stop added.
- KT-014 - multi-stop build was exact (coffee 6, office
  +25, home +30, final order read back). One wrinkle: the
  turn-0 summary already narrated the drive "past the
  coffee shop" before any stop was requested - the staged
  context leaking into the opening summary. Harmless here,
  worth watching.

## Notes for the next revision (not done silently here)

1. Harness change made for this run, flagged per the
   rescore precedent: the Rust shims corpus list
   (shim_load_contexts) gained the key-tests pack as a
   fifth path. Without it the shim delivers no world
   context for KT scenarios and frames every channel as
   text. One path added, comment updated; cargo 54/54.
2. Align the action vocabulary: the spec says open_maps /
   give_directions; the shim system prompt exemplifies
   start_navigation, and the model follows the prompt.
   Pick one vocabulary and put it in both places.
3. Fix KT-009 / KT-013 contexts (active_navigation
   contradicts the opening request).
4. Confirmation discipline for inferred destinations is
   the brains open defect (KT-012, KT-004). If it is
   addressed in the system prompt, this pack is the
   regression test: the two scenarios must pass WITHOUT
   their transcripts getting worse anywhere else.
