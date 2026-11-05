# Cross-check 2 — "what are we missing?" (Gemini, Grok, Perplexity), 2026-10-24 ~4:00 PM PT

Prompt: `EXPANSION-PROMPT.md`. Gemini (Flash, conversation "Building an AI-Native Microkernel Phone OS"),
Grok (Fast, "Redox OS LLM Voice-First Phone Port Gaps…", 75 sources), Perplexity (search b56f943d, 167 sources).
Claims below are theirs; ✔ = verified locally by us, ⚠ = suspect/contradicted.

## Corrections to our plan (Perplexity, strongest)
- **U-Boot is GPL-2.0 overall**; only some files carry BSD-3 headers (✔ we verified `clock-sdm845.c`,
  `spmi-msm.c` are BSD-3). Audit per file — never treat U-Boot as a permissive donor wholesale.
- **edk2-sdm845 → edk2-msm, BSD-2 *plus* a `GPLDrivers/` directory** — exclude/audit it.
- **OpenBSD qc\* drivers target SC7180 / SC8280XP / X1E80100**, not SDM845 → register-family references; per-file
  ISC headers + dependencies must be logged.
- **FreeBSD dwc3 is the xHCI host path**, not a gadget driver. **Embassy CDC-NCM sits on DWC2**, not DWC3 → the
  DWC3 device-mode controller + Qualcomm QSCRATCH glue is ours to write (LWN 2025-06-10 on qcom role signalling).
- `fdt` crate is MPL-2.0; `dtb-walker` is MIT. uefi-rs is MIT OR Apache-2.0 (relicensed Mar 2025).
- Slint is not blanket MIT (royalty-free excludes embedded) → **OrbClient (MIT) first** for the native shell.
- LLM/Rust translation of GPL code is **not** clean-room; keep a per-file **provenance ledger**.

## New product layers we lacked (Perplexity)
Typed **service IR** (triggers, actions, deps, permissions, state schema, tests, UI, power budget) ·
**deterministic executor** (routines run a validated graph, no LLM per trigger) · **transaction model**
(reversible vs committed external effects, idempotency keys) · **service simulator** (fake time/location/battery/
network) · **conflict resolver** · **rescue interface** (non-LLM launcher + physical-key recovery) ·
**capability broker** with scoped, expiring, revocable, audited grants (`audio.capture(while-visible, expiry)`,
`power.request-wake(deadline, max)`) — generated services never get raw driver schemes, shell, fs, or firmware ·
**versioned hot-reload** (candidate → migrate state → health check → atomic switch → keep rollback generation) ·
**registry package** = hash + signature + provenance + WIT interfaces + requested caps + state schema/migrations
+ tests/fixtures + UI schema + power budget + lockfile · thermal/charging/recovery **outside agent authority**.

## New risks (all three)
SMMU/DMA ownership (DMA broker early; physical vs IOVA; never give generated services DMA/MMIO) · firmware memory
map → one boot manifest · **clock/reset/regulator/interconnect dependency DAG** instead of a flat driver list ·
boot-framebuffer lifetime (does scanout survive ExitBootServices/idle?) · USB PHY + Type-C + **charging safety**
(gauge ≠ charger; deterministic limits before unattended charging) · ADSP audio as many boundaries · thermal/DVFS
non-LLM emergency throttle · UFS corruption (RAM root → read-only → writable; EDK2 docs warn of UFS loss) · vendor
firmware EOL Jan 2022 (experimental device, not a secrets vault) · **cellular scope decision** (modem, SIM, calls,
SMS, VoLTE, emergency, GPS, BT, camera, sensors) · DSP firmware via **PIL/PAS SCM calls** (Gemini) · **RPMh votes**
before high-speed peripherals or the SoC freezes (Gemini).

## New projects to copy from
| Project | License | Take |
|---|---|---|
| Goose (block/goose) | Apache-2.0 | Rust agent harness: recipes/sub-recipes, extensions, providers, sessions — base for "talk → tools → routine" |
| Open Interpreter (new Rust, 2026-07-13) | Apache-2.0 | execution/approval/sandbox protocol (NOT the AGPL classic) |
| wasmi / TinyWasm | MIT OR Apache | no_std Wasm engine for generated routines (host functions = only OS path) |
| wasmCloud (wash-runtime) | Apache-2.0 | workload lifecycle, capability providers, component packaging |
| dtb-walker | MIT | no_std DTB traversal (MIT alternative to MPL `fdt`) |
| OrbClient | MIT | first native Redox shell/timeline/waveform UI |
| Thesys OpenUI | Apache-2.0 | streaming UI DSL the model emits token-by-token (Gemini) |
| Leon | MIT | skill/bridge/routine separation (Gemini) |
| Hermes Agent (Nous) | MIT | skills registry + gateway + learning loop (Grok; we already run it) |
| Pipecat (via PearlOS) | BSD-2 | real-time voice pipeline patterns (Grok) |
| CORE (RedPlanetHQ) | check | memory + multi-surface gateway (Grok) |
| NetBSD rump kernels | BSD | user-space BSD drivers behind schemes (Gemini, Grok) |
| whisper.cpp, sherpa-onnx | MIT / Apache | STT/VAD/TTS; avoid GPL phonemizer/FFmpeg utils |
Study-only: AgentOS (AGPL), Genode DDE (AGPL/GPL), Veda (hobby Rust capability microkernel + agent service).

## Suspect claims (verify before use)
⚠ Gemini: OpenBSD `qcrpmh.c` (not in our clone), Fuchsia GENI UART from a Pixel 3 Zircon port, Slint is fine,
FreeBSD dwc3 gadget. ⚠ Grok: "U-Boot BSD-2/3" (contradicted). No source found a non-Linux OS on a OnePlus 6 beyond
Sajna-style Redox login (other phone) and Windows via Renegade.
