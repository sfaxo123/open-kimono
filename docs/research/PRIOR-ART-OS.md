# Prior art for a language-native, Lego-style phone OS (Redox / OnePlus 6)

Researched 2026-10-24. Repo facts (license SPDX, last push, stars) were read live from the GitHub API on
that date; descriptions come from repo READMEs/descriptions. "Last activity" = last push. Items marked
**VERIFY** could not be confirmed from a primary source in this pass (mostly license text GitHub could not
classify). Nothing here is legal advice; re-read each LICENSE before copying code.

Bottom line first: no one has built "LLM as OS layer on a non-Linux microkernel phone". Closest are
Android-based (OpenPhone, Claw OS-style desktop agent OSes). The reusable value is in **protocols and
schemas** (A2UI, AG-UI, MCP Apps, OTel GenAI, Wyoming, OVOS bus), not whole systems. Redox is MIT, so
MIT/Apache/BSD sources are directly compatible.

## 1. LLM / agent operating systems, "AI OS", AI phones

| Project | URL | License | Last activity | Maturity |
|---|---|---|---|---|
| AIOS (agiresearch) | github.com/agiresearch/AIOS | **VERIFY**: GitHub says NOASSERTION, LICENSE file is near-empty; PyPI `aios-core` listed MIT (third party, 2024) | 2026-07-20 (v0.3.0 tag 2026-01-22), 6.5k stars | Academic; runs on top of a host OS |
| OS-Copilot / FRIDAY | github.com/OS-Copilot/OS-Copilot | MIT | 2024-09-09 (stale) | Research prototype |
| Open Interpreter | github.com/openinterpreter/openinterpreter | Apache-2.0 | 2026-10-11, 68k stars | Mature, active |
| Open Interpreter 01 (voice device) | github.com/openinterpreter/01 | **AGPL-3.0** | 2024-11-01 (dormant) | Abandoned-ish |
| Herald OS | github.com/iamlukethedev/Herald-OS | MIT | 2026-10-23, 555 stars | Desktop agent shell on Hermes Agent; young |
| Claw OS | github.com/xiaoyu-work/claw-os | Apache-2.0 (Rust) | 2026-10-16, 12 stars | "Agent runs as privileged system service", dev-stage |
| OpenPhone (Secondly) | github.com/secondly-com/OpenPhone | **PolyForm Noncommercial** (README badge; GitHub NOASSERTION) | 2026-08-18, 203 stars | Dev preview, LineageOS 23.2 / Android 16, Pixel 9a |
| Hermes Agent (Nous) | github.com/NousResearch/hermes-agent | MIT | 2026-10-24, 257k stars | Mature agent runtime |
| OpenClaw | github.com/openclaw/openclaw | MIT | 2026-10-24, 393k stars | Huge; local self-hosted assistant |
| ZeroClaw | github.com/zeroclaw-labs/zeroclaw | Apache-2.0 (Rust) | 2026-10-24, 33k stars | "Fast, small, any OS" agent infra in Rust; **relevant to a Redox target** |
| AgentOS paper (arXiv 2603.08938) | arxiv.org/abs/2603.08938 | paper (CC-BY) | 2026-03 | Concept: NL-driven data ecosystem replacing app silos |
| AOHP, Android Open Harness Project (arXiv 2606.23449) | arxiv.org/abs/2606.23449 | paper | 2026-06 | OS-level agent harness on AOSP |

What to reuse:
- **AIOS**: kernel-style abstractions: agent scheduler, context manager (snapshot/restore LLM context),
  memory manager, tool manager, access manager. Take the *design vocabulary and syscall list* for our
  "model scheduler" daemon; its Cerebrum SDK shows an agent-facing API. License unclear: copy ideas, not code.
- **Open Interpreter** (Apache-2.0): the "computer API" pattern and the run-code-with-confirmation loop;
  safe-mode prompts.
- **OpenPhone**: the best *design* reference (system agent as privileged component, framework-mediated
  actions instead of UI scraping, approvals, active runs, watchers, commitments, audit logs/trajectories).
  License is non-commercial, so read docs only, do not copy code.
- **Claw OS / Herald OS**: how a "talk to the machine, it inspects processes/logs" agent exposes system
  state to a model; Herald's permission-gate-on-mutation model. Both small; read for structure.
- **ZeroClaw / Hermes Agent**: ZeroClaw is the likeliest Rust agent core that could be ported to Redox
  (relibc). Hermes Agent already runs on the homelab, so a remote brain is trivial.

## 2. Voice-first assistant platforms (skills / intents / routines)

| Project | URL | License | Last activity | Maturity |
|---|---|---|---|---|
| OpenVoiceOS ovos-core | github.com/OpenVoiceOS/ovos-core | Apache-2.0 | 2026-10-22, 297 stars | Active Mycroft successor |
| ovos-plugin-manager / ovos-bus-client / ovos-workshop | github.com/OpenVoiceOS/ovos-* | Apache-2.0 | 2026-10-22/24 | Active, small |
| OVOS skills store policy | github.com/OpenVoiceOS/OVOS-skills-store | skills must be OSI-approved | n/a | Community registry |
| Home Assistant core (Assist, assist_pipeline) | github.com/home-assistant/core | Apache-2.0 | 2026-10-24, 92k stars | Very mature |
| Home Assistant intents | github.com/OHF-Voice/intents | **CC-BY-4.0** (data, attribution required) | 2026-10-21 | Mature multilingual sentence templates |
| Wyoming protocol | github.com/OHF-Voice/wyoming | MIT | 2026-09-29 | Stable, tiny |
| linux-voice-assistant | github.com/OHF-Voice/linux-voice-assistant | Apache-2.0 | 2026-10-24 | Active satellite |
| Piper TTS (new) | github.com/OHF-Voice/piper1-gpl | **GPL-3.0** | 2026-10-23 | Active; old rhasspy/piper is MIT but last push 2025-08 |
| Rhasspy 3 | github.com/rhasspy/rhasspy3 | MIT | 2023-12 (archived) | Dead, ideas live on in Wyoming |
| Mycroft core | github.com/MycroftAI/mycroft-core | Apache-2.0 | 2024-09 (archived) | Dead; OVOS superseded it |
| Leon | github.com/leon-ai/leon | MIT | 2026-10-24, 17.6k stars | Active; "personal assistant", skills and tools |
| Willow | github.com/HeyWillow/willow | Apache-2.0 | 2026-10-01, 3.1k stars | ESP32 voice endpoint + inference server |

What to reuse:
- **OVOS messagebus**: every component (STT, intent, skill, TTS) talks over one JSON message bus with
  `type/data/context`. This is almost exactly what a Redox "event" scheme should expose, and it is already
  the "see everything" tap point (listen to the bus = see the whole system). Copy the message-type
  taxonomy (`recognizer_loop:utterance`, `speak`, `mycroft.skill.handler.start/complete`).
- **OVOS plugin manager**: entry-point plugin typing (STT/TTS/WakeWord/VAD/intent) as the pattern for
  swappable "blocks".
- **Wyoming**: line-delimited JSON header + binary payload over a socket; maps 1:1 to a Redox scheme/
  socket and lets satellite/ remote STT/TTS/wake services plug in. Tiny, MIT, copy outright.
- **HA Assist**: pipeline = wake -> STT -> intent -> TTS as a *named, editable pipeline object*; intent
  sentence templates (`turn on [the] {name}`) as the fast no-LLM path before falling to the LLM. Intents
  data is CC-BY-4.0, usable with attribution.
- **Leon**: skill + "tool" + "widget" split, and its on-device-first design. Read for skill manifest shape.
- **Willow**: wake-word/endpoint to server streaming design; low-latency pipeline numbers.

## 3. Generative / on-the-fly UI and sandboxes

| Project | URL | License | Last activity | Maturity |
|---|---|---|---|---|
| A2UI (Google) | github.com/a2ui-project/a2ui (was google/A2UI) | Apache-2.0 | 2026-10-24, 16.8k stars | v0.9.1 stable, v1.0 RC; renderers Lit/Angular/Flutter |
| AG-UI protocol | github.com/ag-ui-protocol/ag-ui | MIT | 2026-10-24, 16.8k | Active event protocol agent <-> frontend |
| CopilotKit | github.com/CopilotKit/CopilotKit | MIT | 2026-10-24, 38k | Mature generative-UI React kit |
| assistant-ui | github.com/assistant-ui/assistant-ui | MIT | 2026-10-24, 12.7k | Mature chat UI primitives incl. tool UIs |
| MCP-UI | github.com/MCP-UI-Org/mcp-ui | Apache-2.0 | 2026-09-16, 5.2k | Feeds MCP Apps; client/server SDKs |
| MCP Apps (ext-apps) | github.com/modelcontextprotocol/ext-apps | Transitioning MIT -> Apache-2.0 (LICENSE says so) | 2026-10-23, 3k | Official MCP extension, `ui://` resources in sandboxed iframes |
| Vercel AI SDK (generative UI/ streaming) | github.com/vercel/ai | Apache-2.0 | 2026-10-24, 27k | Mature |
| OpenAI apps SDK examples | github.com/openai/openai-apps-sdk-examples | MIT | 2026-04-15 | Examples |
| Wasmtime | github.com/bytecodealliance/wasmtime | Apache-2.0 | 2026-10-24 | Mature WASM runtime (Rust) |
| Extism | github.com/extism/extism | BSD-3-Clause | 2026-10-09 | Plugin framework over WASM |
| WIT / component model | github.com/bytecodealliance/wit-bindgen, WebAssembly/component-model | Apache-2.0 (+LLVM exception) | 2026-10 | Typed interfaces between sandboxed components |
| E2B | github.com/e2b-dev/E2B | Apache-2.0 | 2026-10-24 | Cloud sandboxes for agent code (homelab-side idea) |
| smolagents | github.com/huggingface/smolagents | Apache-2.0 | 2026-10-22, 30k | "Agents write code" pattern, small |

What to reuse:
- **A2UI**: the most important one. Agent emits *declarative JSON* naming components from a client-held
  trusted catalog (no code execution), streamed incrementally, with data-model binding and action events
  back. For us: a Redox-native renderer (Orbital/own toolkit) implements the catalog; the LLM builds
  "apps" as A2UI documents. Safe by construction, shareable as plain JSON.
- **AG-UI**: event stream vocabulary (run started, text delta, tool call start/args/end, state delta,
  run finished). Doubles as the wire format for the "see everything" feed.
- **MCP Apps / MCP-UI**: when a block needs real HTML UI, the sandboxed-iframe + postMessage JSON-RPC
  contract. Mostly relevant if we embed a browser; lower priority on Redox.
- **WASM (Wasmtime + Extism + WIT)**: the sandbox for user/LLM-written logic. Each generated service
  becomes a WASM component with declared imports (capabilities). Needs verification that wasmtime builds
  on Redox (not checked); a pure interpreter such as wasmi is the fallback.
- **smolagents**: code-as-action loop with restricted interpreter.

## 4. Composable / "Lego" OS designs

| Project | URL | License | Last activity | Notes |
|---|---|---|---|---|
| Redox OS | github.com/redox-os/redox | MIT | 2026-09-27, 16.7k | Our base: schemes, userspace daemons |
| Redox relibc / kernel / orbital / ion | github.com/redox-os/* | MIT | 2026-09 | libc, kernel, GUI, shell |
| Genode + Sculpt OS | github.com/genodelabs/genode (archived mirror; moved to codeberg.org/genodelabs/genode) | **AGPL-3.0 + linking exception** | mirror archived 2026-05 | Runtime-composable components, "depot" packages |
| Plan 9 (Foundation) | github.com/plan9foundation/plan9 | MIT | 2025-10-28 | Everything-is-a-file namespaces |
| 9front | github.com/9front/9front | none declared on GitHub (MIT/Lucent mix, **VERIFY**) | 2026-10-23 | Active Plan 9 fork |
| Fuchsia components | fuchsia.dev (source fuchsia.googlesource.com) | BSD-3 / Apache-2.0 mix, **VERIFY** | active | Component manifests (CML), capability routing |
| Nix / NixOS | github.com/NixOS/nixpkgs | MIT | 2026-10-24 | Declarative, reproducible, rollback |

What to reuse:
- **Sculpt**: the idea of a *runtime* that composes a system graph of components with explicit routes at
  runtime (config as a live "runtime" document, depot of signed packages). AGPL: study, do not copy.
  This is the closest to "model edits the system graph while it runs".
- **Fuchsia CML**: component manifest = declared `use`/`offer`/`expose` of capabilities; the parent routes
  capabilities to children. Perfect shape for an LLM-writable manifest and for permission display.
- **Plan 9 namespaces**: per-process mount tables; give each generated service its own namespace (only the
  schemes it was granted). Redox schemes + a per-process scheme namespace do this natively.
- **Redox schemes**: a trace scheme (`trace:`/`events:`) and a service-control scheme (`svc:` start/stop/
  status/describe) are the substrate for visibility and power management.
- **Nix**: services/routines as declarative, hashed, shareable derivations; generations for instant
  rollback of an LLM's bad change. Our "recipes as code" shareability maps directly.
- **ADR-like hardware profiles**: NixOS modules/ device trees as model for "hardware profiles".

## 5. Agent observability / transparency ("see everything")

| Project | URL | License | Last activity | Notes |
|---|---|---|---|---|
| OpenTelemetry GenAI semantic conventions | github.com/open-telemetry/semantic-conventions | Apache-2.0 | 2026-10-23 | `invoke_agent` -> `chat` -> `execute_tool` spans; status: Development |
| OpenLLMetry (Traceloop) | github.com/traceloop/openllmetry | Apache-2.0 | 2026-10-19, 7.5k | OTel instrumentation for LLM libs |
| Opik (Comet) | github.com/comet-ml/opik | Apache-2.0 | 2026-10-24, 22.9k | Trace UI + evals |
| Langfuse | github.com/langfuse/langfuse | MIT except `ee/` dirs (ClickHouse) | 2026-10-24, 36k | Trace/session/observation data model |
| Arize Phoenix | github.com/Arize-ai/phoenix | **Elastic License 2.0** (not OSI open source) | 2026-10-24 | UI reference only |
| OVOS messagebus | see section 2 | Apache-2.0 | active | Whole-system event tap |

What to reuse:
- **OTel GenAI names** as our trace schema, so the on-phone feed, the homelab and any viewer
  (Langfuse/Opik) interoperate: spans `invoke_agent`, `chat`, `execute_tool {name}`, attributes for tool
  args/results (opt-in for privacy; for us default on in the local view, redacted on share).
- **Langfuse data model** (trace > observation > generation/span/event, session, score) for a compact
  on-device store; look at UI layout of trace timeline + tree.
- **Opik / Phoenix UIs**: tree + timeline + raw I/O pane for the visual design. Phoenix is read-only
  inspiration (ELv2).
- Add non-LLM spans the others lack: mic state, VAD, STT partials, power transitions, service start/stop.

## 6. Mobile agents operating phones by language

| Project | URL | License | Last activity | Notes |
|---|---|---|---|---|
| Mobile-Agent (X-PLUG) | github.com/X-PLUG/MobileAgent | MIT | 2026-07-07, 9.3k | v1-v3, GUI-Owl models, multi-agent |
| AppAgent | github.com/TencentQQGYLab/AppAgent | MIT | 2025-03-19, 6.9k | Learn-by-exploring doc generation |
| DroidRun / Mobilerun | github.com/droidrun/mobilerun | MIT | 2026-10-24, 9.8k | ReAct agent over ADB + accessibility Portal app |
| Open-AutoGLM | github.com/zai-org/Open-AutoGLM | Apache-2.0 | 2026-03-06, 26k | "Open phone agent model and framework" |
| AgentCPM-GUI | github.com/OpenBMB/AgentCPM-GUI | Apache-2.0 | 2026-01-11 | On-device GUI agent model, compact action schema |
| UI-TARS-desktop | github.com/bytedance/UI-TARS-desktop | Apache-2.0 | 2026-10-09, 39k | GUI agent stack |
| Microsoft UFO | github.com/microsoft/UFO | MIT | 2026-10-23, 10k | Windows UI agent |
| AndroidWorld | github.com/google-research/android_world | Apache-2.0 | 2026-10-23 | Benchmark + task definitions |
| android_env | github.com/google-deepmind/android_env | Apache-2.0 | 2026-10-24 | Action/observation spec for Android |
| playwright-mcp | github.com/microsoft/playwright-mcp | Apache-2.0 | 2026-10-22, 38k | Accessibility-tree-as-observation pattern |

What to reuse:
- **Action schemas**: tap/swipe/type/back/home/open_app/wait/finish (AndroidWorld/android_env,
  AgentCPM-GUI JSON actions, DroidRun tool set). Since we own the OS we should NOT scrape pixels; instead
  expose the same verbs as *typed system tools* and keep the GUI-action verbs only for any Android-compat
  layer. Take the schema, the "finish/ask_user/need_help" terminal actions, and the step-trace format.
- **AppAgent**: "explore once, write down what each element does" doc-generation -> analogous to the OS
  auto-documenting a service block for later reuse.
- **AndroidWorld** tasks: ready-made eval suite of natural-language tasks to port to our tool set.

## Gaps (nobody open has this)
- LLM-edited live system graph on a microkernel (closest: Sculpt runtime, but no LLM; OpenPhone, but Android
  and non-commercial).
- Shareable "block" format combining manifest + WASM + A2UI view + recipe. Compose from the pieces above.
- Power-aware component scheduling explained to the user.

## Top 10 to copy from (ranked; MIT/Apache/BSD first)

1. **A2UI** (Apache-2.0). First thing: adopt the v0.9 JSON message schema and write the minimal component
   catalog (text, button, list, input, card, row/column, toggle, chart) plus a renderer in whatever
   toolkit Redox gets running; make "apps" = A2UI documents the model emits and stores.
2. **OpenTelemetry GenAI semantic conventions** (Apache-2.0). First: define the trace event schema as
   `invoke_agent` / `chat` / `execute_tool` spans with our extra mic/STT/power span types; one JSON-lines
   file as the trace store.
3. **OVOS ovos-core / bus-client** (Apache-2.0). First: copy the messagebus message shape and
   utterance->intent->skill->speak message type list as the Redox event scheme; use the bus as the single
   tap feeding the "see everything" view.
4. **Wyoming** (MIT). First: implement its framed event protocol (JSON header + payload) for the STT/TTS/
   wake-word services so local and homelab engines are interchangeable.
5. **AG-UI** (MIT). First: take its event type list (text delta, tool call start/args/end, state delta,
   run lifecycle) as the UI-facing stream and map A2UI actions onto it.
6. **Fuchsia component manifests / capability routing** (BSD-3/Apache, VERIFY). First: write our block
   manifest format modeled on CML `use/offer/expose` so a model can author it and the UI can show
   permissions before start.
7. **Wasmtime + WIT + Extism** (Apache-2.0 / BSD-3). First: prove a hello-world WASM component (or wasmi
   as fallback) runs on Redox, then define a WIT world for "service" with granted-scheme imports.
8. **Home Assistant Assist + intents** (Apache-2.0 core, CC-BY-4.0 data). First: copy the pipeline object
   model (wake -> STT -> intent -> TTS, named and editable) and ship a handful of intent templates as the
   zero-LLM fast path for "turn off wifi" class requests.
9. **DroidRun/Mobilerun + AndroidWorld/android_env** (MIT / Apache-2.0). First: lift the tool verb list
   and observation/action JSON schema into our system-tool definitions, and port 20 AndroidWorld-style
   tasks as an acceptance suite.
10. **Nix (nixpkgs/NixOS, MIT) generations model + Langfuse data model (MIT)**. First: design block
    install/modify as immutable, content-hashed generations with one-command rollback; and adopt Langfuse's
    trace/observation/session shape for the on-device store. (Honorable: ZeroClaw as the Rust agent core to
    port to Redox; Open Interpreter for the confirm-before-run loop.)

Study but do not copy code: Genode Sculpt (AGPL-3.0 + exception), OpenPhone (PolyForm Noncommercial),
Open Interpreter 01 (AGPL-3.0), Piper new (GPL-3.0), Arize Phoenix (ELv2), AIOS (license unclear).
