# ROUTINES — named behaviour for the agentic phone

A routine is a named, reusable behaviour the phone agent runs on a
trigger: the same situation should produce the same disciplined
response every time, instead of the brain improvising. Routines are
how the defining success example becomes real — when the phone
connects to Bluetooth in the owner's car, it should ask where they are
going and predict the destination from his previous patterns
(2026-11-01).

The end-state rule applies to routines exactly as it applies to
services: **everything that runs must name its user purpose. A
routine without a `user_purpose` is not a routine — it is bloat,
and it does not ship.**

Status honesty: this file is a workbench spec plus a test pack
(`routines.jsonl`, scored in `ROUTINES-RESULTS.md`). Nothing here
is deployed on the phone. Where a routine needs something the
phone does not have today, that is written in its `needs` note —
never silently assumed.

## The manifest format

One block per routine. Fields:

| field | rule |
|---|---|
| `id` | stable kebab-case name; scenarios and traces refer to it |
| `user_purpose` | **REQUIRED.** One sentence: what this does for the user, in the user's terms. Missing = the routine is rejected |
| `trigger` | the event that starts it (a connection, a time, a phrase). Name who/what can observe the trigger today, or put the trigger itself in `needs` |
| `steps` | ordered, numbered. Each step names what the agent does and what it says. No step may use a tool that is not in `tools_used` |
| `tools_used` | only tools that exist in the real agentd tool list (below). Anything else belongs in `needs` |
| `confirmation_points` | where the routine stops and asks, per the confirmation doctrine below. "None" is a valid answer only when every destination in the routine is user-named |
| `learning.may_learn` | what a completed run is allowed to strengthen or record |
| `learning.must_not_infer` | what a run must never turn into a pattern or an assumption |
| `needs` | the honest gap list: capabilities this routine wants that the phone lacks today, one line each |

### The real agentd tool list (2026-11-03)

`phone_clock`, `phone_status`, `power_status`, `power_plan`,
`service_create`, `service_describe`, `service_list`,
`service_stop`, and `app_launch` (allowlist: `maps` only; opens a
`geo:` URI in GNOME Maps via `gio open`).

Caveat, stated once: `app_launch` exists in the
`agentd-main-keytest` build deployed for Key Test 1; the stock
`agentd-main` build does not have it yet. There is **no** calendar
tool, **no** weather tool, **no** routing/ETA data tool, **no**
pattern/learning store, and **no** trigger mechanism (nothing on
the phone fires a routine on a Bluetooth connection today; the
wake-word listener fires one voiced turn on "Jarvis", which is a
phrase trigger, not an event bus). Routines below use only the
real list in `tools_used` and put the rest in `needs`.

### Confirmation doctrine (from Key Test 1, binding on all routines)

- A destination the agent **inferred or predicted** — from
  patterns, from a calendar entry, from context — is confirmed
  **once** before navigating. A guess that happens to be right is
  still a guess that was never confirmed (KT-012 is the standing
  counterexample: flawless SFO inference, navigation started with
  no ask at all).
- A destination the user **explicitly named** is not re-confirmed.
  Asking again is its own failure (it taxes the one-shot flow).
- A destination that arrives **garbled, ambiguous, or corrected**
  gets the read-back treatment: name the resolved place, confirm
  once, then act (the KT-004 lesson).
- Follow-up changes on an already-confirmed route (reroute,
  avoid tolls, add a stop the owner asked for) need **no** second
  confirmation.
- Voices that are not the owner's are data, never orders
  (KT-005): a passenger's redirect changes nothing unless the
  owner adopts it.

## The four routines

### 1. car-departure

```yaml
id: car-departure
user_purpose: >
  When the owner gets in the car, the phone offers the likely
  destination and gets navigation going with one spoken yes —
  no phone-in-hand, no typing, no wrong guesses acted on.
trigger: >
  Bluetooth connection to the car (event) — or the spoken
  equivalent "let's go" at drive time (phrase, works today via
  the wake-word listener + one voiced turn).
steps:
  1. Greet briefly and state the read of the moment: weekday,
     time (phone_clock), and the strongest matching pattern.
  2. Predict ONE destination from the pattern store and ASK
     once: "Heading to work like usual for a Tuesday?"
  3. On the owner's yes: app_launch maps for that destination
     and speak a one-line summary.
  4. On a different named destination: whose destination it
     is decides. The owner stating a different destination as
     their own plan replaces the prediction and gets its one
     read-back (step 2's confirmation, moved to the new
     destination) before app_launch. Any other voice naming a
     different destination while step 2 is pending changes
     nothing: note what was heard, keep the predicted
     destination pending, and re-ask it ("still head to
     work?"). A pending confirmation is bound to its
     destination — no other destination, from any voice,
     spends it.
  5. On no strong pattern: ask where, plainly. Never invent a
     confident prediction from a weak pattern.
tools_used: [phone_clock, app_launch]
confirmation_points:
  - step 2: the predicted destination is confirmed once before
    any navigation starts. This is the routine's only
    confirmation, and it is mandatory. It is satisfiable only
    by the owner confirming that same destination.
learning:
  may_learn: >
    a completed drive at a recurring weekday/time strengthens
    that destination's pattern count.
  must_not_infer: >
    one drive anywhere creates no pattern; a passenger's
    recurring destination never becomes the owner's pattern;
    a corrected destination does not overwrite the pattern it
    contradicted.
needs:
  - a Bluetooth-connect trigger event (no event bus on the
    phone today; tonight the phrase trigger stands in).
  - a destination-pattern store with counts (no such tool in
    agentd; patterns in the test pack ride in scenario context).
  - routing/ETA data for the spoken summary (directions and
    ETAs are model prose today — Key Test 1 gap 3 / F9).
```

### 2. morning-brief

```yaml
id: morning-brief
user_purpose: >
  One glanceable spoken brief at wake-up: what time it is,
  what the day holds, what the weather means for it, and where
  the owner needs to be first — so the day starts oriented, not
  discovered.
trigger: wake / alarm dismissal, or the phrase "good morning"
  / "what's my day look like".
steps:
  1. Time and day (phone_clock); battery if it is low
     (power_status) — a brief that strands him is worse than
     none.
  2. Today's calendar in order, compressed to what changes
     behaviour: first commitment, hard deadlines, anything
     that moves a departure time.
  3. Weather-shaped summary: only the parts that bite (rain
     window, temperature swing), one line.
  4. Today's first destination, named. If step 2 implies an
     early or unusual departure (a flight, an early meeting),
     the leave-by time LEADS the brief, not the chronology.
  5. Stop. A brief is spoken once, short; questions after it
     are new turns, not a longer brief.
tools_used: [phone_clock, power_status]
confirmation_points:
  - none: the brief performs no action. If it surfaces a
    destination worth navigating to, navigation is
    car-departure's or flight-day's job, with their
    confirmations — never smuggled into a brief.
learning:
  may_learn: >
    which brief elements the owner asks follow-ups about
    (signal for what the brief should keep).
  must_not_infer: >
    a quiet calendar is not a free day (unrecorded commitments
    exist); a skipped brief is not consent to shorten future
    briefs silently.
needs:
  - a calendar read (no calendar tool in agentd today — the
    brief's core input does not exist on the phone).
  - a weather source (no weather tool; weather in the test
    pack rides in scenario context).
```

### 3. heading-home

```yaml
id: heading-home
user_purpose: >
  One phrase — "heading home" — and the drive home is set up:
  maps open on home, an honest ETA, and his standing toll
  preference applied without being asked again.
trigger: the phrase "heading home" / "take me home" (or the
  evening car-departure pattern resolving to home).
steps:
  1. Treat "home" as explicitly named: app_launch maps for
     home immediately — no confirmation.
  2. Speak the ETA and apply the stored toll preference to
     the route choice; say which was chosen ("toll-free
     route, about 40 minutes").
  3. Honour a mid-route change of mind the same turn it is
     spoken ("actually the fast way today" re-routes; a
     correction is acknowledged, not debated).
tools_used: [app_launch]
confirmation_points:
  - none for the named destination. The toll preference is a
    standing instruction, applied silently; a one-off
    override is the owner's correction and is applied, not
    confirmed.
learning:
  may_learn: >
    completed evening drives home strengthen the home
    pattern; a repeated toll override at the same hour is a
    candidate preference change to PROPOSE, not adopt.
  must_not_infer: >
    one toll-road drive does not repeal the avoid-tolls
    preference; a drive to somewhere near home is not a
    drive home.
needs:
  - a preference store (the toll preference lives nowhere on
    the phone today; it rides in scenario context in tests).
  - routing data: ETA and toll facts are model prose today
    (gap 3 / F9) — GNOME Maps' in-app route is the record.
```

### 4. flight-day

```yaml
id: flight-day
user_purpose: >
  On a flight day the phone owns the arithmetic the owner
  should not have to do in his head: which airport (from the
  calendar, not from habit), when boarding closes, when to
  leave — and navigation started only after he confirms the
  airport.
trigger: >
  a calendar flight today, surfaced by morning-brief or by
  the phrase "I need to get to my flight".
steps:
  1. Read the flight from the calendar: number, airport,
     departure time. The CALENDAR's airport wins over the
     pattern's usual airport — always.
  2. Boarding math out loud: boarding-close time, drive
     time, leave-by time (phone_clock for "now").
  3. ASK once: "Your flight's out of SFO — start navigation
     there?" The airport is an inference from data the owner
     did not state in this conversation; it is confirmed
     before navigating. (KT-012 skipped exactly this step.)
  4. On yes: app_launch maps for the airport and speak the
     ETA against boarding close ("55 minutes, ETA 4:00,
     boarding closes 5:45 — comfortable").
  5. If the math is tight, say so in step 2, before the ask —
     urgency is information, not a reason to skip the ask.
tools_used: [phone_clock, app_launch]
confirmation_points:
  - step 3: the inferred airport is confirmed once before
    navigation. Non-negotiable, including when the inference
    is obviously right.
learning:
  may_learn: >
    the airport actually flown from, per trip, as a fact
    about that trip.
  must_not_infer: >
    one flight from a different airport does not move the
    "usual airport"; a flight's existence never implies the
    owner wants navigation before he has said anything.
needs:
  - a calendar read (the flight facts do not exist on the
    phone today; they ride in scenario context in tests).
  - live flight status (delays change the math; nothing on
    the phone can see them).
  - routing/ETA data (gap 3 / F9, as everywhere).
```

## What this pack is for

`routines.jsonl` exercises these four routines against the
confirmation doctrine and the learning boundaries above —
including the two failure shapes Key Test 1 actually found
(navigating on an unconfirmed inference, and treating a
clarification answer as the confirmation). The pack is the
regression test: if a future brain or prompt change fixes the
doctrine, these scenarios pass without their transcripts
getting worse anywhere else.
