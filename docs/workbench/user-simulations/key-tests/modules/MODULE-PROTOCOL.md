# On-the-fly modules — manifest protocol v1 (workbench prototype)

Status: PROTOTYPE, workbench only. Nothing here runs on the
phone. Adoption path is at the bottom.

## The idea

Instead of fixed apps, the phone agent composes small,
single-purpose modules (cards) on the fly for the task at
hand: an ETA card during a drive, a flight card on flight
day. A module is a **manifest** — declarative JSON data,
never code — that a renderer turns into one card. The agent
fills the manifest's inputs from what it already knows; the
card can only offer actions the agent already has.

This follows the project's minimalism rule: anything
running without a stated user purpose is bloat. So every
manifest must name its purpose, and a manifest without one
is invalid, not merely undocumented.

## Manifest fields

A module definition is one JSON object:

| field | type | required | meaning |
|---|---|---|---|
| `id` | string, kebab-case, unique | yes | Stable name, e.g. `eta-card`. |
| `protocol` | string | yes | Protocol version this manifest targets. `"1"` here. |
| `user_purpose` | string, one sentence | **REQUIRED** | The user-facing job this module exists to do, in the user's terms ("Show the drive time home while the user is driving"). A renderer or agent MUST refuse a manifest with a missing or empty `user_purpose`. One module = one purpose; a second purpose is a second module. |
| `trigger` | object | yes | When this module is relevant. `trigger.when` is a short declarative description (e.g. `"navigation_active"`); `trigger.context` lists the context keys the agent checks (e.g. `["active_route"]`). Triggers are evaluated by the agent/host, never by the module. |
| `inputs` | array of input specs | yes | The typed values the card displays. See below. |
| `layout` | array of blocks | yes | The card's visual structure, in order, using ONLY the closed block vocabulary below. |
| `actions` | array of action specs | yes (may be empty) | The buttons the card may offer. Each action names ONE real agentd tool. See below. |
| `lifetime` | object | yes | `appears_when` (the condition that brings the card up), `expires_when` (the condition that removes it), and `max_ttl_minutes` (a hard cap; the card dies at the cap even if the expiry condition never fires). An expired module renders nothing and offers no actions. |
| `sample_inputs` | object | no | Example values for every input, for mocks and tests. Not shipped to the renderer in production. |

### Inputs

Each input spec: `{ "name": ..., "type": ..., "required": bool,
"source": ... }`.

- `type` is one of: `string`, `number`, `minutes`
  (a number of minutes), `time` (24h `"HH:MM"`), `boolean`.
- `source` is one of: `agent` (the agent computed or looked
  it up), `context` (already in the session context, e.g.
  the active route), `tool` (a named tool's result — the
  tool must appear in `actions` or already be part of the
  agent's own reasoning; the module never calls it).
- Inputs are values, not expressions. No formulas, no
  string templates with logic, no references to other
  modules. If a value needs computing, the agent computes
  it before filling the input.

### Layout — the closed block vocabulary

Exactly four block types exist in protocol v1. A renderer
MUST reject a manifest using any other block type.

| block | fields | renders as |
|---|---|---|
| `title` | `text` (an input name or a literal) | The card's heading line. |
| `big_number` | `value` (input name), `unit` (literal), `caption` (literal or input name) | One large figure with its unit — the glanceable fact. |
| `rows` | `items`: array of `{label, value}` (value = input name or literal) | Label/value lines. |
| `action_button` | `label`, `action` (an `id` from `actions`) | A tappable button wired to exactly one declared action. |

No other blocks. No styling fields, no images, no links,
no free-form HTML. If a card cannot be said with these
four blocks, it is not a v1 module.

### Actions

Each action spec: `{ "id": ..., "tool": ..., "args": {...} }`.

- `tool` MUST be one of the real agentd tools (2026-11-03):
  `phone_clock`, `phone_status`, `power_status`,
  `power_plan`, `service_create`, `service_describe`,
  `service_list`, `service_stop`, `app_launch`.
  An action naming anything else is invalid, and the
  renderer must refuse the whole manifest — a card never
  invents a capability.
- `app_launch` is further limited by that tool's own
  allowlist: tonight `args.app` may only be `"maps"`.
- `args` values are input names or literals. The tool is
  invoked by the agent/host through the normal broker
  rules (class, confirmation, journaling) exactly as if
  the user had asked in conversation — the card changes
  the presentation, never the permissions.
- A button may only reference an action declared in the
  same manifest. No action outside the manifest exists.

## Hard rules

1. **Declarative only.** A manifest is data. It contains
   no code, no scripts, no expressions, no callbacks.
2. **No network fetch by the module.** A module never
   fetches anything. Every value on the card arrived as
   an input the agent filled.
3. **No action outside the manifest, no tool outside the
   real tool list.** The manifest is the complete set of
   things the card can do.
4. **`user_purpose` is required and singular.** Missing,
   empty, or multi-purpose manifests are invalid.
5. **Lifetime is bounded.** Every module expires by its
   condition or its TTL cap, whichever comes first.
6. **The renderer treats the manifest as untrusted
   data.** Input values are displayed as text, never
   interpreted as markup or commands.
7. **Broker rules are unchanged.** Confirmations,
   journaling, and refusals behave exactly as they do
   for the same tool invoked conversationally.

## Adoption path (not part of this prototype)

For modules to appear on the phone, a renderer must exist:
the natural home is kimono-ui rendering manifests the
agent posts — that is Track A work and needs Claude's and
the owner's direction. Until then this protocol, the three
test manifests, and `mock.html` are a paper prototype:
they prove the shape is expressible with the real tools
and the KEY TEST 1 data, nothing more.
