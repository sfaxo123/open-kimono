# Validation — on-the-fly module prototype

Run 2026-11-03 on the 3090, against the files in this
directory (`eta-card.json`, `flight-card.json`,
`maps-card.json`, `mock.html`, `MODULE-PROTOCOL.md`).
Nothing here was run on the phone; the phone was not
contacted for this prototype.

## 1. Every JSON parses

Command (run in this directory):

```sh
for f in eta-card.json flight-card.json maps-card.json; do
  python3 -m json.tool $f > /dev/null && echo "$f: JSON parses OK"
done
```

Output:

```text
eta-card.json: JSON parses OK
flight-card.json: JSON parses OK
maps-card.json: JSON parses OK
```

## 2. Every manifest field maps to the protocol

Each JSON carries exactly the fields `MODULE-PROTOCOL.md`
defines — the eight required fields plus the optional
`sample_inputs`:

| protocol field | eta-card | flight-card | maps-card |
|---|---|---|---|
| `id` | `eta-card` | `flight-card` | `maps-card` |
| `protocol` | `"1"` | `"1"` | `"1"` |
| `user_purpose` | drive time at a glance | when to leave for the airport | one-tap open of a discussed place |
| `trigger` | `navigation_active` | `calendar_flight_today` | `destination_discussed` |
| `inputs` | destination, minutes, route, toll_note | flight, departs, boarding_closes, airport, eta_minutes, leave_by | destination |
| `layout` | title, big_number, rows | title, big_number, rows, action_button | title, action_button |
| `actions` | (none — display only) | `app_launch` | `app_launch` |
| `lifetime` | route end / 240 min cap | departure / 720 min cap | opened-or-moved-on / 30 min cap |
| `sample_inputs` | KT-001 values | KT-012 values | San Francisco (smoke-proven) |

## 3. Conformance + action existence, checked mechanically

Checker: `~/work/validate_modules.py` (scratch, not part
of the deliverables). It verifies, per manifest: all
required fields present; `user_purpose` non-empty; input
types inside the protocol's type set; every required
input has a sample value; every layout block inside the
closed vocabulary (`title`, `rows`, `big_number`,
`action_button`); every `action_button` references an
action declared in the same manifest; every action's
`tool` is in the REAL agentd tool list (2026-11-03:
`phone_clock`, `phone_status`, `power_status`,
`power_plan`, `service_create`, `service_describe`,
`service_list`, `service_stop`, `app_launch`); every
`app_launch` action has `args.app == "maps"` (the tool's
own allowlist); `lifetime.max_ttl_minutes` is set.

Output:

```text
eta-card.json: fields=['actions', 'id', 'inputs', 'layout', 'lifetime', 'protocol', 'sample_inputs', 'trigger', 'user_purpose'] blocks=['title', 'big_number', 'rows'] actions=[]
flight-card.json: fields=['actions', 'id', 'inputs', 'layout', 'lifetime', 'protocol', 'sample_inputs', 'trigger', 'user_purpose'] blocks=['title', 'big_number', 'rows', 'action_button'] actions=[('open_airport_maps', 'app_launch')]
maps-card.json: fields=['actions', 'id', 'inputs', 'layout', 'lifetime', 'protocol', 'sample_inputs', 'trigger', 'user_purpose'] blocks=['title', 'action_button'] actions=[('open_maps', 'app_launch')]
CONFORMANCE: PASS - all checks
```

So the only actions any card can offer are `app_launch`
calls with `app = "maps"` — a tool that exists, with the
exact argument its allowlist accepts. The eta-card is
display-only and declares no actions at all.

## 4. The mock is one static file with the same data

`grep -c 'http://\|https://\|<script\|<link' mock.html`
returns **0** — no external assets, no scripts, no
frameworks; styling is one inline `<style>` block. The
values rendered in the cards are the manifests'
`sample_inputs`, verified present in the HTML: SFO (×2),
55 (×2), I-880, "No tolls on the fast route", UA482 from
SFO, 16:50, 18:30, 17:45, "Open in Maps" (×2). Sample
data provenance: KT-001 (airport ETA 55 min via I-880,
no tolls), KT-012 (UA482, departs 18:30, boarding closes
17:45, 55-minute drive; leave-by 16:50 = boarding close
minus drive, no buffer added), and the KEY TEST 1
addendum's smoke-proven destination, San Francisco.

## Honest limits

- **This is a rendering mock.** Nothing here runs on the
  phone, no renderer exists, and no manifest has ever
  been displayed by kimono-ui or any other software.
- The conformance checker is a one-off scratch script,
  not a schema enforced anywhere in agentd or the UI.
- Trigger evaluation, input filling, lifetime expiry,
  and broker-mediated action invocation are all
  described in the protocol but implemented nowhere —
  they are host/agent behaviour the protocol assumes.
- The flight-card's `leave_by` is arithmetic on sample
  data, not a live computation.
- **Adoption path:** kimono-ui rendering manifests the
  agent posts is Track A work. It needs Claude's and
  the owner's direction; this prototype is built so the
  Track A side can adopt or reject it as-is.
