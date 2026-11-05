# UX Specification — The Minimal Open-Kimono View (`docs/design/UX.md`)

**Task ID:** D3 (Wave 1)  
**Target OS:** Redox OS on OnePlus 6 (SDM845, `enchilada`) — Language-Native Operating Layer (Part B1, B7, B10, B11)  
**Inputs:** `docs/VISION.md` (esp. §§1–8), `docs/PLAN.md`, `docs/design/SERVICE-IR.md`, `crates/trace-events`, `op6-refs/{a2ui,openui,ag-ui}`, and the postmarketOS voice-app baseline in `~/artifacts/handoff/oneplus-demo-v1/w6a6-shots/` (`A01`–`A14`).  
**License:** MIT (clean-room specification; references `ag-ui` MIT, `openui` MIT, `a2ui` Apache-2.0, `otel-semconv` Apache-2.0, `orbclient` MIT).

---

## 1. Design Philosophy & Visual System

The user interface of the language-native Redox phone OS is **one clean screen** rendered via `orbclient` / `tiny-skia` over the `display.vesa` scheme (`1080×2280` physical pixels on the Samsung `sofef00-ams628nw01` AMOLED panel, `360×760` logical dp at `3x` scale).

### 1.1 Core Invariants (`docs/VISION.md` §§1–8)
1. **No app grid, no hidden background clutter (`VISION.md` §1, §8):** There is no home screen of static icons, no notification shade full of marketing telemetry, and no opaque background daemons. If nothing is running for the user's current intent, the screen is pure black (`#000000` — zero AMOLED pixel current) with a single status strip and the voice/text bar.
2. **Open kimono — every event and its *reason* is visible (`VISION.md` §3, §4):** The UI is a pure projection of the `trace:` scheme (`crates/trace-events`) plus active declarative surfaces (`a2ui` / `openui`). Whenever a microphone opens or closes, a capability grant is issued or revoked, a hardware rail or Redox scheme daemon powers on or off, or a tool call executes, the UI renders **what** happened and **why** (`reason` field).
3. **Honest and never silent (`VISION.md` §5):** Cloud or homelab hand-offs (`5090`, `Spark`, `3090`) are announced before or as they start, display live round-trip latency and target host, and remain cancellable. When the system cannot fulfill a request, it renders an explicit refusal card stating what failed, which constraint blocked it, and what local alternative exists.
4. **Everything visible is editable (`VISION.md` §2):** Every rendered surface, widget, and active routine block carries an addressable semantic ID (`surface_id`, `component_id`, `service_id@gen`). Speaking *"change that routine"* or tapping any element enters an atomic candidate-generation flow (`generation: N+1`) with simulator verification and one-tap rollback.

### 1.2 Palette, Typography & Spatial Grid (from `w6a6-shots/A01–A14`)

| Token | Hex / Value | AMOLED / UX Role |
|---|---|---|
| `bg.canvas` | `#000000` | True AMOLED black background (0 mA pixel draw when idle) |
| `bg.card` | `#0A0A0A` | Elevated card / trace drawer / dock surface (`A06`, `A09`, `A14`) |
| `bg.pill` | `#141414` | Secondary button fill, resource badge, and chip background (`A01`, `A13`) |
| `border.subtle` | `#242424` | `1 dp` (3 px) hairline card and button border |
| `accent.orange` | `#FF6B00` | Primary active indicator: live mic ring, waveform bars, primary CTA (`Start`, `[ MIC ON ]`, `DONE`, `ERROR` banner in `A10`) |
| `accent.orange_dim` | `#4D2200` | Completed timer digits (`00:00` in `A12`), decaying wake-lock progress fill, dimmed waveform baseline |
| `text.primary` | `#FAFAFA` | Primary transcript, cleaned text, hero metrics (`10:00` in `A01`), button labels |
| `text.secondary` | `#9E9E9E` | Reasoning stream, tool arguments, `reason` strings, voice hint subtitles |
| `text.muted` | `#666666` | Monospace timestamps (`20:18:52`), sequence numbers, inactive badges |
| `font.sans` | `Inter / Noto Sans` | Headings (`18 dp` bold), body/transcript (`15 dp` regular), hero numbers (`56 dp` bold) |
| `font.mono` | `JetBrains Mono` | Trace log lines (`12 dp`), resource/power badges, latency counters, JSON pointers |

### 1.3 Single-Screen Spatial Architecture (`360×760` dp Logical Viewport)

```
+--------------------------------------------------+  y = 0 dp
| ZONE 1: TRUTH BAR (36 dp)                        |  Identity / Nav, Mic LED, Net Bytes, Power/CPU
+--------------------------------------------------+  y = 36 dp
| ZONE 2: HAND-OFF / REFUSAL BANNER (0 or 44 dp)   |  Collapsed (0 dp) unless Handoff or RunError active
+--------------------------------------------------+
|                                                  |
| ZONE 3: PRIMARY STAGE (flex: 320–520 dp)         |  Mode A (Default): Open-Kimono Live Stream
|   - Either Open-Kimono Live Stream:              |    (Transcript -> Cleaned -> Reasoning -> Tools)
|     * Utterance: SttPartial -> SttFinal          |  Mode B (Surface Mounted): Declarative A2UI / OpenUI
|     * Cleaned Text (with raw diff toggle)        |    mini-app or routine inspector (A01, A04, A13)
|     * Streaming Reasoning & OTel GenAI Spans     |
|     * Tool Call Cards & Results                  |
|   - Or Mounted A2UI / OpenUI Surface             |
|                                                  |
+--------------------------------------------------+
| ZONE 4: SERVICES, POWER & TRACE DRAWER           |  Collapsed: 1-line strip of active services + power
|   (36 dp collapsed / 180 dp expanded as in A06)  |  Expanded (`>`): timestamped `trace:` journal lines
+--------------------------------------------------+
| ZONE 5: PERSISTENT VOICE + TEXT DOCK (96 dp)     |  Mic toggle ring (`[ MIC ON ]`), 24-bar orange
|   (Anchored bottom dock from A01/A08/A10/A14)    |  waveform, status + live partial, `[ ASK / EDIT ]`
+--------------------------------------------------+  y = 760 dp
```

---

## 2. Core Open-Kimono Elements & Behavioral Rules

### 2.1 Mic State & Waveform (`Op6Event::MicStarted`, `MicFrame`, `MicStopped`)
* **Hardware-Verified Indicator:** The microphone state is never inferred from UI button presses alone; it updates strictly when `trace:` emits `Op6Event::MicStarted { device }` or `Op6Event::MicStopped { reason }`, corroborated by the audio scheme's active stream counter (`streams 1` vs `streams 0 (verified)` as proven in `w6a6-shots/A09` and `A14`).
* **Visual States in Zone 1 & Zone 5:**
  * **LIVE (`LISTENING`):** Solid `#FF6B00` dot inside an orange ring (`(●)`), 24 vertical orange bars driven at 30–60 Hz by `Op6Event::MicFrame { level: 0.0..=1.0 }`, label `LISTENING` in `#FF6B00`.
  * **GATED (`SPEAKING · MIC GATED`):** When TTS playback (`audio.playback.tts`) is active (`A06`, `A12`), the mic automatically gates to prevent self-triggering; the dock shows `SPEAKING · MIC GATED` with a low-amplitude cadence bar and logs `tts playing · mic gated`.
  * **CLOSED (`OFF`):** Muted grey ring `(○)`, flat dotted baseline `··············`, label `OFF · mic closed · <reason>` (`A08`, `A09`). Closing the mic is **never silent**: `Op6Event::MicStopped { reason }` (e.g., `"user tap"`, `"utterance complete"`, `"grant expired"`, `"rescue kill-switch"`) is rendered directly in the dock subtitle.

### 2.2 Live Transcript → Cleaned Text (`SttPartial`, `SttFinal`, `TextCleaned`)
1. **Streaming Partial (`Op6Event::SttPartial`):** As the user speaks, partial words appear in Zone 5's dock preview (`okay gemma, se...` in `A01`) and in Zone 3's active utterance card in `#9E9E9E` with an animated orange cursor `▍` and an engine badge (`[stt: moonshine-small]` or `[stt: whisper-tiny]`).
2. **Final Raw Transcript (`Op6Event::SttFinal`):** Locks the raw string in `#FAFAFA` and attaches the measured STT latency badge (`142 ms`) and confidence score if `< 0.85`.
3. **Disambiguated Cleaned Text (`Op6Event::TextCleaned { raw, cleaned }`):** Replaces filler words, wake-phrase prefix (`"okay gemma"`), and speech disfluencies with the normalized intent string.
   * **Zero-Clutter Diff Rule:** If `cleaned` differs from `raw`, the primary line displays `cleaned` in bold `#FAFAFA` while a single muted sub-line shows `raw: "okay gemma um set a timer for ten minutes"` (`#666666`). Tapping the sub-line reverts to `raw` or opens inline text editing.

### 2.3 Streaming Reasoning & Tool Calls (`AgUiEvent` + `GenAiEvent`)
* **Reasoning Block (`AgUiEvent::ReasoningStart` → `ReasoningMessageContent` / `ReasoningMessageChunk` → `ReasoningEnd`):**
  * Rendered inside a left-bordered (`2 dp` `#FF6B00` while streaming, `#242424` once finished) monospace-header block in Zone 3.
  * Paired with `GenAiEvent::SpanStart` / `SpanEnd` sharing `span_id`: the header shows the OTel span name (`chat qwen3-4b-local` or `chat spark/deepseek-v3`), time-to-first-chunk (`gen_ai.response.time_to_first_chunk`), and live token counter (`gen_ai.usage.input_tokens` → `gen_ai.usage.output_tokens`).
  * Encrypted reasoning state (`AgUiEvent::ReasoningEncryptedValue`) renders a compact `[🔒 encrypted-thought: <subtype> · <entityId>]` pill rather than dumping opaque ciphertext.
* **Tool Call Cards (`AgUiEvent::ToolCallStart` → `ToolCallArgs` → `ToolCallEnd` → `ToolCallResult` & `GenAiOperation::ExecuteTool`):**
  * Each tool call renders as a compact 2-line pill inside Zone 3:
    * **Line 1:** `⚙ <toolCallName>` + streaming JSON arguments (`ToolCallArgs.delta`) + execution spinner or duration (`18 ms`).
    * **Line 2:** Result summary (`ToolCallResult.content`) or error badge (`SpanStatus::Error`). Tapping expands the full JSON payload.

### 2.4 Active Services with Resource/Power Badges (`ServiceIR` + `Op6Event`)
Every running `ServiceIR` block (`docs/design/SERVICE-IR.md` §2) and every powered hardware component appears in Zone 4's **Active Services & Power Strip**:
* **Service Pill Format:** `[svc.timer.countdown v1 · 1 Silver · 4/8 MiB · 1 wk/m · 0 B net]`
  * Derived from `Op6Event::ServiceStarted { service, generation, reason }` + `ServiceIR.power_budget` (`cpu_cluster_preference`, `memory_limit_mb`, `max_wakeups_per_minute`, `network_egress_bytes_max`).
* **Capability & Wake-Lock Countdown Badges:**
  * `Op6Event::GrantGranted { service, capability, scope, expires_us }` and `Op6Event::WakeLockAcquired { holder, deadline_us, max_us }` render a live decaying orange progress bar showing remaining TTL (e.g., `[wake: svc.timer · 14s / 30s max]`).
* **Power Switching Announcements (`Op6Event::PowerChange`):**
  * Whenever a Redox scheme daemon or hardware domain switches (`PowerState::On` or `PowerState::Off`), Zone 4 flashes the `reason` string verbatim (e.g., `⚡ gps:gnss ON — reason: grant location.gnss.read for svc.nav.offline_route`, followed by `⚡ gps:gnss OFF — reason: surface nav_main dismissed`).

### 2.5 Network-Use Indicator (`VISION.md` §8)
* **Offline-First Default:** The top Truth Bar (Zone 1) permanently includes a network cell. When all active services have `network_egress_bytes_max == 0` and no `network.connect` grant exists, it displays `NET: OFF (0 B)` in `#666666`.
* **Live Egress Transparency:** The instant `Op6Event::GrantGranted { capability: "network.connect", scope: Some("powerhouse-mesh:4000"), .. }` fires, the indicator turns `#FF6B00`: `NET: ↑ 1.4 KB · powerhouse-mesh:4000 · TTL 12s`. Tapping it immediately revokes the `network.connect` grant.

### 2.6 Homelab Hand-Off Notice (`Op6Event::Handoff`, `VISION.md` §5)
When a task exceeds on-device capacity (or local STT/LLM delegates to the homelab over USB CDC-NCM / Wi-Fi mesh), `Op6Event::Handoff { destination, reason, status }` activates **Zone 2 (Hand-Off Banner)**:
* **`HandoffStatus::Started`:** Displays an orange-bordered banner:
  `⇄ HAND-OFF → 5090 (powerhouse-mesh:4000 · 41ms) | Reason: synthesizing new ServiceIR wasm | [ CANCEL ]`
* **`HandoffStatus::Completed`:** Collapses into a 1-line receipt in the trace drawer (`20:19:04 handoff ✓ 5090 · 680ms`).
* **`HandoffStatus::Failed`:** Transitions immediately into an **Honest "Can't Do That" Card** (Section 2.7) — never hangs or retries silently.

### 2.7 Honest "Can't Do That" States (`VISION.md` §5, `w6a6-shots/A10`)
When an action fails or is refused (`AgUiEvent::RunError`, `SubagentError`, `Op6Event::GrantRevoked`, `Op6Event::Handoff { status: Failed }`, or client `VALIDATION_FAILED`), the UI renders an **Honest Refusal Card** with four mandatory fields:
1. **Status Pill (`#FF6B00` fill, `#000000` bold text, matching `A10`):** `CAN'T DO THAT · <CODE>` (e.g., `CAPABILITY_DENIED`, `OFFLINE_POLICY`, `HW_NOT_PORTED`, `E-APP-NO-ANSWER`).
2. **What Was Requested:** Plain-English summary of the attempted action.
3. **Exact Reason (`why`):** Cites the specific boundary (e.g., *"Service `svc.nav.offline_route` has `network_egress_bytes_max = 0` and cannot fetch live traffic tiles from `tile.openstreetmap.org`"* or *"Modem QMI voice call scheme (`modem:voice`) is not yet active on this boot"*).
4. **Actionable Recovery Buttons:** Concrete local choices (e.g., `[ Use Offline Cached Maps ]`, `[ Grant 1-Time Network (60s) ]`, or `[ Dismiss ]`).

---

## 3. Lifecycle of Generated UIs (A2UI v0.9/v1.0 & Thesys OpenUI)

Generated and library mini-apps (`Timer` in `A01`, `Counter` in `A04`, `Water Tracker` in `A11`, `Weekly Hydration Checklist` in `A13`) are declarative surfaces rendered inside **Zone 3 (Primary Stage)** while **Zone 4** and **Zone 5** remain anchored at the bottom.

### 3.1 How Surfaces Appear & Stream Into View
The UI surface host supports both declarative protocols specified in `docs/design/SERVICE-IR.md` §1.1 & §2.1:

1. **A2UI JSON Protocol (`op6-refs/a2ui/specification/v0_9/json/server_to_client.json` & `openui/packages/a2ui/src/protocol-schema.ts`):**
   * **`createSurface`:** Allocates a named surface slot (`surfaceId`, `catalogId`, optional `theme: {"primaryColor": "#FF6B00"}`, `sendDataModel`). Renders the Zone 1 surface header: `← Back   <Title>   [v<gen> · generated | library]`.If `generation == 1` and newly synthesized, adds an orange `NEW` pill (`A11`).
   * **`updateComponents`:** Validates component nodes against the basic catalog (`root` component required; rejects `UNALLOWED_PARENT` / `UNALLOWED_CHILD` with a client-to-server `VALIDATION_FAILED` error event) and mounts the widget tree in Zone 3.
   * **`updateDataModel`:** Applies RFC 6901 JSON Pointer updates (`path: "/remaining_sec"`, `value: 599`) bound via `@path` expressions without re-laying out the component tree.
2. **Thesys OpenUI Streaming Statement Patch (`op6-refs/openui/packages/a2ui/src/statement-patch.ts`):**
   * While an LLM or `ServiceIR` `ui_patch` action streams `openui_statements` (`$var = Component(...)`), the host invokes `mergeComponentStatements(existing, deltaStatements)`:
     * Statements are keyed by identifier (`$root`, `$header`, `$timerDigits`, `$buttonRow`) in declaration order.
     * Temporarily unreachable statements are buffered without error so streaming top-down or bottom-up works smoothly; as soon as `$root` references a child `$id`, it appears on screen with a `120 ms` border highlight.
     * Setting `$id = null` deletes that component cleanly in place.

### 3.2 User Interaction & Client-to-Server Return Path
* Touching a button (`+1 min`, `Start`, `Clear done`) or speaking a bound voice command (`"add a minute"`, `"reset"` as shown in `A01`/`A04` voice hint footers) emits:
  1. An A2UI `action` message (`{ "version": "v1.0", "action": { "name": "timer.add", "surfaceId": "surf_timer", "sourceComponentId": "btn_plus_1m", "timestamp": "...", "context": { "seconds": 60 } } }`) routed directly to the `ServiceIR` `ui_action` trigger (`zero-LLM` fast path).
  2. A corresponding trace line in Zone 4 (`20:18:52 app command · timer.add 60 s` in `A06`).

### 3.3 How Surfaces Get Dismissed & Power Down Dependencies
A surface in Zone 3 is dismissed by any of four paths:
1. **Voice command:** Speaking `"back"`, `"close"`, or `"dismiss"` (handled deterministically by `ovos.stop` / surface navigation matcher, logged as `20:35:01 "back"` → `20:35:02 app closed · Timer` in `A14`).
2. **Touch action:** Tapping `← Back` in the top-left of Zone 1.
3. **Programmatic completion:** Receiving `{"version": "v1.0", "deleteSurface": {"surfaceId": "..."}}` from the service when an ephemeral task finishes.
4. **Hardware key:** Short-pressing `Vol Down + Vol Up` chord returns to the root Open-Kimono stream.

**Dismissal Power Rule (`VISION.md` §4 & §8):**
* If the service holds only `while_visible` / `while_surface_visible` capability grants, dismissing the surface immediately triggers `Op6Event::GrantRevoked { reason: "surface dismissed" }` → `Op6Event::ServiceStopped` → `Op6Event::PowerChange { state: Off }`.
* If the service has an active background trigger or countdown (`standing_trigger_watch` or `until_deadline`, e.g., a running 10-minute timer or the `< 20%` battery watcher), dismissing the UI surface destroys the view buffer (`0 MiB` UI RAM) while condensing the service into a 1-line pill in **Zone 4** (`[svc.timer.countdown · 06:12 left · 1 Silver]`). Tapping that pill re-mounts the surface from `state:`.

---

## 4. Voice & Text Editing of Any Visible Element ("Change That Routine")

Every visible item on screen—whether a generated UI button, a running timer, or a standing background routine in Zone 4—can be inspected and modified on the fly without opening an IDE or restarting the OS (`VISION.md` §2, `PLAN.md` B8).

### 4.1 Referential Resolution ("That Routine" / "This Button")
The UI maintains a lightweight **Visible Context Table** injected into the agent session metadata whenever the user speaks or taps `ASK`:
* `focused_surface_id`: e.g., `surf_hydration` or `None`
* `visible_services`: e.g., `["svc.power.low_battery_alert@v1.0.0#gen1", "svc.timer.countdown@v1.0.0#gen2"]`
* `selected_element_id`: set if the user long-presses any widget, badge, or trace row (`250 ms` hold draws a `#FF6B00` dashed focus box around the target element and opens the edit bar pre-scoped to that element).

### 4.2 Atomic Edit → Simulator Gate → Hot-Reload Flow
1. **Trigger:** User says *"Change that battery routine so it warns me at 15% instead of 20% and adds a quiet-hours check"* (or taps the `[svc.power.low_battery_alert]` pill in Zone 4 and types in the `ASK` bar).
2. **Live Diff Card in Zone 3:** The agent synthesizes candidate generation `gen: 2` (`ServiceIR` patch or OpenUI statement patch) and renders a human-readable diff before committing:
   * `- triggers[0].sensor_threshold.threshold: 20`
   * `+ triggers[0].sensor_threshold.threshold: 15`
   * `+ triggers[0].sensor_threshold.hysteresis_reset: 18`
   * `+ triggers[0].condition_cel: "env.local_hour >= 7 && env.local_hour < 23"`
3. **Deterministic Simulator Verification (`SERVICE-IR.md` §2.1 `tests`):** Before switching generations, the executor runs the package's `SimulatorTestCase` suite in an isolated memory store. Zone 3 displays: `✓ Simulator: 3/3 tests passed · 0 B net · 2 MiB RAM`.
4. **Atomic Generation Switch & Rollback Pill (`PLAN.md` B8):**
   * State is migrated via `state_schema.migrations` on `state:`, `ServiceStarted { generation: 2 }` is emitted, and the header badge updates to `[v1.1 · gen 2]`.
   * For 60 seconds (and permanently in the service drawer), a `[ ↺ Rollback to gen 1 ]` action pill is displayed; speaking *"undo that"* or tapping the pill atomically restores `gen 1` and its snapshot.

---

## 5. Rescue (Non-LLM) Screen (`PLAN.md` B11)

The **Rescue Interface** is a zero-LLM, zero-Wasm, zero-network recovery and control screen compiled as a tiny standalone binary (`rescued`) that talks directly to `display.vesa`, `inputd` (`spmid` PM8998 PON/GPIO keys + `rmi4d` touch), `sys:power`, and the supervisor scheme.

### 5.1 Deterministic Entry & Physical-Key Navigation
* **Hardware Chord (Works Even If UI/LLM/Touch Is Frozen):** Holding **Power (`PM8998 PON`) + Volume Down (`PM8998 GPIO 5`)** for `1500 ms` (handled in `spmid` / supervisor outside agent authority) preempts the framebuffer and switches to the Rescue Screen.
* **Touch Entry:** Tapping the system identity/power cell (`SYS / PWR`) in the top-right of Zone 1 and selecting `[ RESCUE / HARD CONTROLS ]`.
* **Full Physical-Button Operation (When Touch `rmi4d` Is Unavailable):**
  * **Volume Down (`PM8998 GPIO 5`):** Move selection cursor down (`▶`).
  * **Volume Up (`PM8998 GPIO 6`):** Move selection cursor up (`▲`).
  * **Power Short Press (`PM8998 PON`):** Toggle / execute the highlighted control.
  * **Power Double Press:** Exit Rescue Screen back to Open-Kimono UI.

### 5.2 Rescue Capabilities (Strictly Outside Agent Authority — `PLAN.md` B10, B11)
1. **Hardware Kill Switches:**
   * `[ HARD-KILL MIC ]` — Forces audio capture scheme closed, revokes all `audio.capture` grants.
   * `[ KILL ALL NETWORK / USB-NET ]` — Drops Wi-Fi (`wcn3990`) and/or USB CDC-NCM external routing while optionally keeping local USB debug console (`CDC-ACM` / serial) alive.
   * `[ FREEZE / KILL ALL WASM SANDBOXES ]` — Terminates all `wasmi` instances and resets fuel pools.
2. **Scheme & Service Process Table:** Lists every active `svc.*` and userspace driver daemon (`spmid`, `geni-i2cd`, `rmi4d`, `vesad`, `ufsd`) with live RAM/CPU usage and per-row `[ STOP ]` and `[ ROLLBACK GEN ]` actions.
3. **Thermal, Battery & Panel Safety Readout:** Direct read-only PM8998/bq27411 voltage, current, SoC %, die temperature, charging limit status, and MIPI DCS brightness slider (`10%`–`100%`).
4. **Boot & Console Escape:** `[ OPEN RAMSH TEXT CONSOLE (fbcond) ]` and `[ REBOOT TO FASTBOOT / U-BOOT ]`.

---

## 6. Six Canonical Screen States (ASCII Wireframes)

All wireframes represent the `360×760` dp portrait viewport (`50 chars × 34 lines` monospace grid) using the exact black (`#000000`) + orange (`#FF6B00`, denoted by `●`, `█`, `[!]`, and double-line active borders) visual grammar from `w6a6-shots/A01–A14`.

### State 1: Idle / Listening → Live Transcript → Cleaned Text (Open-Kimono Home)
*Context:* User speaks `"okay gemma um set a timer for ten minutes"`. Mic is live (`Op6Event::MicStarted` + `MicFrame`), STT finalizes (`SttFinal`), text is cleaned (`TextCleaned`), and zero-LLM fast-path intent (`ha-intents` `HassStartTimer`) matches without network traffic.

```
+--------------------------------------------------+
| agent@oneplus   ● LISTENING   NET:OFF  ⚡84% 1C  |
+--------------------------------------------------+
|                                                  |
|  UTTERANCE #42                      20:18:10.140 |
|  +--------------------------------------------+  |
|  | [stt: moonshine-small · 138ms · conf 0.96] |  |
|  |                                            |  |
|  | "Set a timer for 10 minutes"               |  |
|  |                                            |  |
|  | raw: "okay gemma um set a timer for ten    |  |
|  |       minutes"                 [edit text] |  |
|  +--------------------------------------------+  |
|                                                  |
|  INTENT FAST-PATH (0 LLM tokens · 2ms)           |
|  +--------------------------------------------+  |
|  | ✓ matched: HassStartTimer (speech_to_phrase|  |
|  |   slots: { duration_sec: 600 }             |  |
|  | → starting: svc.timer.countdown (gen 1)    |  |
|  |   reason: voice_intent trig_voice_start    |  |
|  +--------------------------------------------+  |
|                                                  |
|                                                  |
+--------------------------------------------------+
| 1 svc: timer.countdown [1 Silv·2MB·0B]  wake:armed|
+--------------------------------------------------+
| +----------------------------------------------+ |
| | (●) █▌██▎▊█▍█▊▎█▌ LISTENING               >  | |
| |     ▌█▍▊▌█▎▌█▍▊▌  "Set a timer for 10 m..."  | |
| +----------------------------------------------+ |
| [     ● MIC ON      ]  [      ASK / TYPE       ] |
+--------------------------------------------------+
```

### State 2: Streaming Reasoning, Tool Calls & Homelab Hand-Off
*Context:* User asks *"Build me a hydration tracker for 2000 ml a day with a +250 ml button"*. On-device router announces a homelab hand-off (`Op6Event::Handoff` to `5090`), streams reasoning (`AgUiEvent::ReasoningMessageContent`), runs a tool call (`ToolCallStart` → `ToolCallResult`), and shows active network egress + expiring wake lock.

```
+--------------------------------------------------+
| agent@oneplus   ○ MIC OFF     NET:↑3KB ⚡83% 2C  |
+==================================================+
| ⇄ HAND-OFF → 5090 (mesh 38ms)            [CANCEL]|
|   Reason: compile new ServiceIR + OpenUI surface |
+==================================================+
|  UTTERANCE #43                      20:20:04.012 |
|  "Build a hydration tracker for 2000 ml a day"   |
|                                                  |
|  REASONING · chat 5090/qwen3-coder (TTFC 94ms)   |
|  ┃ Designing ServiceIR `svc.health.water_tracker`|
|  ┃ with state_schema `/current_ml` (default 0),  |
|  ┃ `/goal_ml` (2000), and OpenUI surface         |
|  ┃ `surf_water` with +250ml, -250ml, reset... ▍  |
|  ┃ [in: 412 tok · out: 189 tok · 1.1s]           |
|                                                  |
|  TOOL CALLS                                      |
|  +--------------------------------------------+  |
|  | ✓ ⚙ simulator.verify_ir (42ms)             |  |
|  |   args: {"id":"svc.health.water_tracker"}  |  |
|  |   result: 2/2 tests PASS · 0B runtime net  |  |
|  +--------------------------------------------+  |
|  | ⟳ ⚙ caps.request_grant (running...)        |  |
|  |   args: {"cap":"state.namespace.readwrite"}|  |
|  +--------------------------------------------+  |
+--------------------------------------------------+
| 2 svc: timer(08:06) · builder [wake 9s/15s max]  |
+--------------------------------------------------+
| +----------------------------------------------+ |
| | (○) ·············· OFF · mic closed         > | |
| |                    reason: utterance complete | |
| +----------------------------------------------+ |
| [      MIC OFF      ]  [      ASK / TYPE       ] |
+--------------------------------------------------+
```

### State 3: Active Generated UI Surface (A2UI / OpenUI) + Expanded Trace Drawer
*Context:* Follows the exact layout of `w6a6-shots/A06`, `A11`, and `A13`: the generated surface is mounted in Zone 3 with its version/provenance pill (`v1.0 · generated`), while the user has tapped `>` on the bottom dock to expand Zone 4's live `trace:` journal drawer.

```
+--------------------------------------------------+
| ← Back  Water Tracker  [v1.0·generated] [EDIT]   |
+--------------------------------------------------+
| WATER TRACKER          [1 Silv·3MB·0B]  [ACTIVE] |
|                                                  |
|                   1000                           |
|          ████████████░░░░░░░░░░░░                |
|            goal: 2000 milliliters                |
|                                                  |
|   +--------+  +------------------+  +---------+  |
|   | -250ml |  |   ██ +250 ml ██  |  |  Reset  |  |
|   +--------+  +------------------+  +---------+  |
|                                                  |
|  Voice: "add 250 ml", "reset", "change this UI"  |
+--------------------------------------------------+
| streams 1 · wake armed · mesh up 41ms · net 0 B  |
| 20:21:10 svc.health.water_tracker gen 1 started  |
| 20:21:10 grant ✓ state:water_tracker (while_vis) |
| 20:21:14 wake ✓ "okay gemma"                     |
| 20:21:16 app command · water.add 250 -> 1000 ml  |
| 20:21:16 state:water_tracker wal commit #4 (1ms) |
+--------------------------------------------------+
| +----------------------------------------------+ |
| | (●) █▎▊█▌▎█▊▍█▌▎▊ LISTENING                < | |
| |     ▌█▎▍█▌▊▎█▍▌▊▎ "add 250 ml · 1000"        | |
| +----------------------------------------------+ |
| [     ● MIC ON      ]  [      ASK / TYPE       ] |
+--------------------------------------------------+
```

### State 4: Voice / Text Editing of a Visible Element ("Change That Routine")
*Context:* While inspecting the standing battery alert routine (`svc.power.low_battery_alert` from `SERVICE-IR.md`), the user says *"Change that routine to warn me at 15% instead of 20% and add a +5% hysteresis"*. The UI displays the live `gen 1 → gen 2` diff, simulator proof, and rollback control.

```
+--------------------------------------------------+
| ← Back  svc.power.low_battery  [gen 1 → gen 2]   |
+--------------------------------------------------+
| EDITING ROUTINE BY VOICE            20:24:02.410 |
| "Change that routine to warn me at 15% with      |
|  +5% hysteresis"                                 |
|                                                  |
| CANDIDATE DIFF (svc.power.low_battery_alert)     |
| +----------------------------------------------+ |
| | @@ triggers[0].sensor_threshold @@           | |
| |   scheme_path: "power:battery/capacity"      | |
| | - threshold: 20                              | |
| | + threshold: 15                              | |
| | - hysteresis_reset: 23                       | |
| | + hysteresis_reset: 20                       | |
| | @@ actions.speak_alert.dialog @@             | |
| | - speech: "Battery is below 20 percent."     | |
| | + speech: "Battery is below 15 percent."     | |
| +----------------------------------------------+ |
|                                                  |
| SIMULATOR GATE (`tests`: 2 fixtures)             |
| +----------------------------------------------+ |
| | ✓ test_drop_below_15_fires: PASS (3ms)       | |
| | ✓ test_bounce_16_to_19_suppressed: PASS (2ms)| |
| | Budget: 1 Silver · 2 MiB · irq_event · 0B net| |
| +----------------------------------------------+ |
|                                                  |
|  [ ██ APPLY GEN 2 ██ ]   [ ↺ KEEP GEN 1 (ESC) ]  |
+--------------------------------------------------+
| 1 svc: low_battery_alert [gen 2 staged · 0B net] |
+--------------------------------------------------+
| +----------------------------------------------+ |
| | (●) ▎▌▊█▍▎▌▊▍▎▌▊  LISTENING · say "apply"  > | |
| |     ▍▎▌▊▎▍▌▊▍▎▌▊  or "undo"                  | |
| +----------------------------------------------+ |
| [     ● MIC ON      ]  [      ASK / TYPE       ] |
+--------------------------------------------------+
```

### State 5: Honest "Can't Do That" / Refusal & Local Fallback State
*Context:* User asks `"Send an SMS to Alice that I'm 10 minutes away"` while on a build where the SDM845 modem QMI SMS scheme (`modem:sms`) is not active and offline policy blocks cloud bridges (`VISION.md` §5, `w6a6-shots/A10`).

```
+--------------------------------------------------+
| agent@oneplus   ○ MIC OFF     NET:OFF  ⚡81% 1C  |
+==================================================+
| [!] CAN'T DO THAT · SCHEME_UNAVAILABLE           |
+==================================================+
|  UTTERANCE #48                      20:26:11.005 |
|  "Send an SMS to Alice that I'm 10 minutes away" |
|                                                  |
|  WHY THIS WAS REFUSED                            |
|  +--------------------------------------------+  |
|  | 1. Scheme `modem:sms` is not running:      |  |
|  |    SDM845 QMI/QRTR modem voice/SMS driver  |  |
|  |    (Part A17) is disabled on this boot.    |  |
|  | 2. No silent fallback: `network.connect`   |  |
|  |    is OFF by default; no message was queued|  |
|  |    or sent over external networks.         |  |
|  +--------------------------------------------+  |
|                                                  |
|  WHAT WE CAN DO RIGHT NOW                        |
|  +--------------------------------------------+  |
|  | [1] Save draft to `state:notes/outbox`     |  |
|  |     (0 B net · local SQLite WAL)           |  |
|  | [2] Hand off via homelab Matrix bridge     |  |
|  |     (requires 1-time `network.connect`     |  |
|  |      grant to `powerhouse-mesh:8008`)      |  |
|  | [3] Dismiss                                |  |
|  +--------------------------------------------+  |
+--------------------------------------------------+
| 1 svc: timer.countdown(02:01) · net: 0 B         |
+--------------------------------------------------+
| +----------------------------------------------+ |
| | (○) ·············· [██ ERROR ██]            > | |
| |                    SCHEME_UNAVAILABLE: modem   | |
| +----------------------------------------------+ |
| [      MIC OFF      ]  [      ASK / TYPE       ] |
+--------------------------------------------------+
```

### State 6: Rescue (Non-LLM) Screen (`PLAN.md` B11)
*Context:* Reached deterministically via hardware chord (`Power PON + Vol Down` 1.5s via `spmid`) or touch. Zero LLM, zero Wasm, zero network dependencies. Navigable by `Vol Up`/`Vol Down` + `Power` or touch (`rmi4d`).

```
+==================================================+
| [!] REDOX RESCUE CONSOLE (NON-LLM · DETERMINISTIC|
| Keys: [Vol▲/▼] Select  [Pwr] Toggle  [2xPwr] Exit|
+==================================================+
| HARDWARE KILL SWITCHES (OUTSIDE AGENT AUTHORITY) |
| ▶ [1] MIC CAPTURE (`audio:`)    [ HARD-LOCKED OFF|
|   [2] ALL NETWORK / MESH        [ HARD-LOCKED OFF|
|   [3] WASM SANDBOXES (`wasmi`)  [ 0 RUNNING · KILL|
|   [4] PANEL BRIGHTNESS (DCS)    [ ████░░░░  45% ]|
+--------------------------------------------------+
| ACTIVE SCHEMES & SERVICES (RAM: 48 / 8192 MiB)   |
|   ID / DAEMON          GEN   CPU/RAM   ACTION    |
|   svc.timer.countdown   g1   0% 2MB   [STOP][g-1]|
|   svc.power.low_batt    g2   0% 1MB   [STOP][g1] |
|   spmid (PM8998 keys)   --   0% 1MB   [OK · IRQ] |
|   rmi4d (s3706b touch)  --   0% 2MB   [OK · I2C] |
|   vesad (cont_splash)   --   1% 12MB  [OK · FB]  |
+--------------------------------------------------+
| HARDWARE & THERMAL TELEMETRY (READ-ONLY)         |
|   Battery: 81% (4.08V, -142mA discharging)       |
|   SoC Temp: 34.2 °C (Kryo Silver0 ON, Gold OFF)  |
|   Boot Mode: RAM-only initfs (slot_a untouched)  |
+--------------------------------------------------+
| [5] OPEN RAMSH SHELL (`fbcond`)                  |
| [6] REBOOT TO FASTBOOT / U-BOOT                  |
| [7] EXIT RESCUE → RETURN TO OPEN-KIMONO UI       |
+==================================================+
```

---

## 7. Event → Widget Mapping Table

Every state transition in the Open-Kimono UI is driven deterministically by an incoming record on `/scheme/trace` (`crates/trace-events`) or an A2UI/OpenUI surface message. No widget mutates hidden state outside this table.

| Source Vocabulary | Event Variant / Message | Key Payload Fields | Target Zone & Widget | Visual Mutation & Open-Kimono Reason Display |
|---|---|---|---|---|
| **`Op6Event`** (`crates/trace-events/src/op6.rs`) | `MicStarted` | `device: Option<String>` | **Zone 1** Mic LED + **Zone 5** Voice Dock + **Zone 4** Trace Drawer | Switches Zone 1 & Zone 5 ring to solid `#FF6B00` `(●) LISTENING`; increments verified stream counter (`streams 1`); appends `mic open · <device>` to Zone 4. |
| **`Op6Event`** | `MicFrame` | `level: f32` (`0.0..=1.0`) | **Zone 5** 24-Bar Waveform | Pushes normalized bar height into the 24-sample ring buffer; renders symmetric vertical orange (`#FF6B00`) bars at 30–60 fps. |
| **`Op6Event`** | `MicStopped` | `reason: String` (mandatory) | **Zone 1** Mic LED + **Zone 5** Voice Dock + **Zone 4** Trace Drawer | Switches ring to muted grey `(○) OFF`, flattens waveform to `······`, and renders `mic closed · <reason>` directly in Zone 5 subtitle (never silent). |
| **`Op6Event`** | `SttPartial` | `engine`, `text`, `confidence` | **Zone 3** Utterance Card + **Zone 5** Dock Subtitle | Updates live streaming transcript text in `#9E9E9E` with trailing orange cursor `▍` and `[stt: <engine>]` badge. |
| **`Op6Event`** | `SttFinal` | `engine`, `text`, `confidence`, `latency_us` | **Zone 3** Utterance Card | Locks raw utterance text in `#FAFAFA` and attaches latency pill (`<latency_us/1000> ms`) and confidence badge. |
| **`Op6Event`** | `TextCleaned` | `raw`, `cleaned` | **Zone 3** Utterance Card | Promotes `cleaned` to primary bold `#FAFAFA` headline; collapses `raw` into a muted `#666666` sub-line (`raw: "..."`) with `[edit text]` action. |
| **`Op6Event`** | `PowerChange` | `component`, `state: On\|Off`, `reason` | **Zone 1** Power/CPU Cell + **Zone 4** Power Strip & Drawer | Updates active hardware/scheme pill (`⚡ <component> ON/OFF`) and flashes `reason` in Zone 4 (`VISION.md` §4). |
| **`Op6Event`** | `WakeLockAcquired` | `holder`, `deadline_us`, `max_us` | **Zone 4** Active Services & Power Strip | Renders an orange countdown pill `[wake: <holder> · <rem>s / <max>s]` that ticks down toward `deadline_us`. |
| **`Op6Event`** | `WakeLockReleased` | `holder`, `reason` | **Zone 4** Active Services & Power Strip | Removes wake-lock pill and logs `wakelock released · <holder> (<reason>)` in Zone 4 drawer. |
| **`Op6Event`** | `ServiceStarted` | `service`, `generation`, `reason` | **Zone 4** Active Services Strip | Adds service badge `[<service> g<generation> · <budget>]` and logs `reason` in the trace drawer. |
| **`Op6Event`** | `ServiceStopped` | `service`, `generation`, `reason` | **Zone 4** Active Services Strip + **Zone 3** (if mounted) | Removes active service badge, unmounts any `while_visible` surface, and logs `stopped · <reason>`. |
| **`Op6Event`** | `GrantGranted` | `service`, `capability`, `scope`, `expires_us` | **Zone 1** (if `network.connect`) + **Zone 4** Service Pill | Attaches capability chip (`cap: <capability> [<scope>]`) with expiry countdown; if `capability == "network.connect"`, turns Zone 1 `NET:` indicator `#FF6B00` with target origin. |
| **`Op6Event`** | `GrantRevoked` | `service`, `capability`, `reason` | **Zone 1** + **Zone 4** + **Zone 2** (if denied mid-run) | Strips capability chip; resets `NET: OFF` if last network grant; if revocation blocked an active user request, opens an Honest Refusal Card with `reason`. |
| **`Op6Event`** | `Handoff` | `destination`, `reason`, `status: Started\|Completed\|Failed` | **Zone 2** Hand-Off Banner + **Zone 4** Trace Drawer | `Started`: expands orange Zone 2 banner showing `destination`, `reason`, mesh RTT, and `[CANCEL]`. `Completed`: collapses banner into trace receipt. `Failed`: transitions to State 5 Refusal Card. |
| **`GenAiEvent`** (`crates/trace-events/src/genai.rs`) | `SpanStart` | `name` (`{op} {target}`), `attributes` (`gen_ai.*`), envelope `span_id` | **Zone 3** Reasoning / Tool Header | Opens a timed span badge (`chat <model>`, `execute_tool <tool>`, `invoke_agent <agent>`) keyed by `span_id` with live elapsed timer. |
| **`GenAiEvent`** | `SpanEnd` | `name`, `status: Ok\|Error`, `attributes` (`usage.*`, `time_to_first_chunk`) | **Zone 3** Reasoning / Tool Header | Stops span timer; renders `TTFC`, `input_tokens → output_tokens`, and marks header `#FF6B00` error if `status == SpanStatus::Error`. |
| **`AgUiEvent`** (`crates/trace-events/src/agui.rs`) | `RunStarted`, `StepStarted`, `SubagentStarted` | `thread_id`, `run_id`, `step_name`, `subagent_run_id`, `name` | **Zone 3** Run Header + **Zone 4** Trace Drawer | Mounts active run/step/subagent breadcrumb (`run:<run_id> ▸ <step_name>`) in Zone 3. |
| **`AgUiEvent`** | `RunFinished`, `StepFinished`, `SubagentFinished` | `outcome` (`Success`, `Interrupt`, `Cancelled`, `Suspended`), `usage` | **Zone 3** Run Header + **Zone 5** Dock | Finalizes run breadcrumb; if `outcome == Interrupt { interrupts }`, renders an explicit approval/confirmation card with `interrupt.reason` and `expires_at`. |
| **`AgUiEvent`** | `RunError`, `SubagentError` | `message`, `code`, `usage` | **Zone 2** Banner + **Zone 3** Refusal Card + **Zone 5** (`[ERROR]`) | Transitions immediately to **State 5 (Honest "Can't Do That" Card)** displaying `code`, `message`, and local recovery options (`A10`). |
| **`AgUiEvent`** | `ReasoningStart`, `ReasoningMessageStart`, `ReasoningMessageContent`, `ReasoningMessageChunk`, `ReasoningMessageEnd`, `ReasoningEnd` | `message_id`, `role`, `delta` | **Zone 3** Streaming Reasoning Block | Streams reasoning tokens into a collapsible, orange-left-bordered block above tool calls and assistant answer. |
| **`AgUiEvent`** | `ReasoningEncryptedValue` | `subtype`, `entity_id`, `encrypted_value` | **Zone 3** Reasoning Block | Renders compact `[🔒 encrypted: <subtype> · <entity_id>]` badge without raw base64 clutter. |
| **`AgUiEvent`** | `TextMessageStart`, `TextMessageContent`, `TextMessageChunk`, `TextMessageEnd` | `message_id`, `role`, `delta` | **Zone 3** Assistant Answer Card + **Zone 5** TTS Preview | Streams assistant response text in `#FAFAFA`; if TTS is active, syncs spoken line in Zone 5 (`SPEAKING · "..."` as in `A12`). |
| **`AgUiEvent`** | `ToolCallStart`, `ToolCallArgs`, `ToolCallChunk`, `ToolCallEnd`, `ToolCallResult` | `tool_call_id`, `tool_call_name`, `delta`, `content` | **Zone 3** Tool Call Stack | Creates tool card keyed by `tool_call_id`, streams JSON arguments (`delta`), and attaches result summary on `ToolCallResult`. |
| **`AgUiEvent`** | `StateSnapshot`, `StateDelta`, `MessagesSnapshot`, `ActivitySnapshot`, `ActivityDelta` | `snapshot`, `delta` (RFC 6902 JSON Patch), `activity_type`, `content`, `patch` | **Zone 3** Mounted Surface Data Model + **Zone 4** State Inspector | Applies snapshot or RFC 6902 patch to the bound surface/session store and highlights mutated keys for `150 ms`. |
| **`AgUiEvent`** | `Raw`, `Custom` | `event` / `name`, `value` | **Zone 4** Expanded Trace Drawer | Appends timestamped diagnostic line `[raw/custom: <name>]` in the monospace trace drawer (`never dropped silently`). |
| **`A2UI` / `OpenUI`** (`op6-refs/a2ui`, `op6-refs/openui`) | `createSurface`, `updateComponents`, `updateDataModel`, `openui_statements` | `surfaceId`, `catalogId`, `components` (JSON or `$var = ...` statements), `path`, `value` | **Zone 1** Nav Header + **Zone 3** Primary Stage | Mounts or patches declarative widget tree in Zone 3 (`State 3`), binds `@path` JSON pointers to `state:`, and shows `[v<gen> · generated]` pill. |
| **`A2UI` / `OpenUI`** | `deleteSurface` / Client `action` / Client `error` | `surfaceId`, `action.name`, `action.context`, `error.code` (`VALIDATION_FAILED`) | **Zone 3** Primary Stage → **Zone 4** Service Pill | `action`: dispatches zero-LLM `ui_action` trigger and logs `app command` in Zone 4. `deleteSurface`: unmounts Zone 3 view and revokes `while_visible` grants. `error`: displays schema validation path in Zone 3. |

---

## 8. Provenance & Reference Attribution

No third-party source code was copied into this design specification (`spec-only`). All referenced protocols and visual baselines carry permissive licenses verified in `docs/REFERENCES-INDEX.md`:

| Upstream Source | Local Reference Path | License | How Used in `docs/design/UX.md` |
|---|---|---|---|
| **AG-UI Protocol (1.0)** | `op6-refs/ag-ui` @ `73b04fd` (`spec/1.0/schema.json`) & `crates/trace-events/src/agui.rs` | MIT | Spec-only: all 31 `EventType` variants mapped to UI zones in Section 7 |
| **OpenTelemetry GenAI SemConv** | `op6-refs/otel-semconv` @ `3e25804` & `crates/trace-events/src/genai.rs` | Apache-2.0 | Spec-only: `SpanStart`/`SpanEnd` and `gen_ai.*` attributes mapped to reasoning/tool headers |
| **Google A2UI (v0.9 / v1.0)** | `op6-refs/a2ui` @ `409ded2` (`specification/v0_9/json/*.json`) | Apache-2.0 | Spec-only: `createSurface`, `updateComponents`, `updateDataModel`, `deleteSurface`, client `action` & `VALIDATION_FAILED` lifecycle |
| **Thesys OpenUI** | `op6-refs/openui` @ `9d73d17` (`packages/a2ui/src/statement-patch.ts`, `protocol-schema.ts`) | MIT | Spec-only: statement-level `$var = ...` streaming UI patches and `$var = null` deletion semantics |
| **Redox OrbClient** | `op6-refs/orbclient` @ `5306851` | MIT | Spec-only: target rendering substrate for `apps/timeline` and `rescued` |
| **OnePlus 6 Voice-App Shots** | `~/artifacts/handoff/oneplus-demo-v1/w6a6-shots/A01–A14` | Project Internal (MIT) | Visual baseline for black + `#FF6B00` orange aesthetic, bottom listening dock, expandable trace drawer (`A06`/`A09`), error badge (`A10`), and generated surface header (`A11`/`A13`) |
