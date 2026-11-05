Provenance: first on-phone LFM3-1.7B test, run 2026-11-03 under brief-lfm3-phone.md; raw records in ~/work/lfm3/ on the 3090 (lfm3-report.json, journal.txt). Adopted by root 2026-11-03 as the Phase M evaluation record (PLAN.md M1.1/M1.2 evidence). Model file on the phone: ~/agentic/models-lfm3/LFM3-1.7B-Q4_K_M.gguf, sha256 3793e1e6e2266919 (root re-verified on the phone at adoption).

# LFM3-1.7B on the OnePlus 6 — first on-phone test (2026-11-03)

Operator question: "did we test this model on the phone?" — No, until
now. LFM3-1.7B was researched 2026-11-02 as the ROG Ally fast-brain
pick (research only). This is the first real on-phone test, run
2026-11-03 ~18:56-20:10 PT under brief-lfm3-phone.md. All phone-side
files are additive under ~/agentic/models-lfm3/; no agentd binary,
config, or deployed model was modified; nothing is left running.

## Model identity

- Repo: LiquidAI/LFM3-1.7B-GGUF (official GGUF repo; the base repo
  LiquidAI/LFM3-1.7B ships no GGUF files)
- File: LFM3-1.7B-Q4_K_M.gguf — the Q4_K_M-class file the brief asked
  for (other official quants: BF16/F16/NVFP4/Q5_K_M/Q6_K/Q8_0/QAD-Q4_0)
- Size: 1,099,627,808 bytes (1.02 GiB on disk; HF x-linked-size and
  the repo API agree)
- sha256: 3793e1e6e2266919d6e015287221b41100175a98eddaba92e111aed3b05c93c4
- Fetch path, honestly: downloaded on the 3090 from Hugging Face and
  hashed there; the Meshnet scp to the phone ran at ~0.5 MB/s and was
  abandoned at ~174 MB; the phone then downloaded the file directly
  from the same HF URL at ~14-20 MB/s, and its sha256 was verified
  byte-identical to the 3090 copy before any test ran.

## Recon (phone, before any change)

- Storage: 71.5 GB free of 106.1 GB. RAM: 7,542 MB total,
  4,550 MB available at idle.
- How the on-device fallback is wired (read from the deployment):
  agentd-kimono.toml points local_url at
  http://127.0.0.1:8080/v1/chat/completions, local_model "gemma3-1b";
  the model file is /opt/models/gemma-3-1b-it-qat-Q4_0.gguf
  (720,425,600 bytes). Nothing was listening on 8080 during recon.
- The llama.cpp binaries live at /opt/voice/llama.cpp/build/bin/
  (build 0693dbf). They are glibc-linked (Debian) aarch64 ELF and
  cannot exec on the musl host directly; the deployment pattern
  (see /opt/voice/.build-run.sh) runs them inside root podman
  containers from docker.io/library/debian:12-slim with /opt/voice
  and /opt/models bind-mounted. Every run below follows that pattern.
  The runtime containers also need libgomp1, which the slim image
  lacks; it was apt-installed inside the throwaway containers only.

## Load test — the phone's own llama.cpp CANNOT load LFM3

llama-bench (phone's build 0693dbf, CPU, 4 threads) on the LFM3 file:

    llama_model_load: error loading model: unknown model architecture: 'lfm3'
    llama_bench: error: failed to load model

Root cause: the phone's llama.cpp build (2026-10-22) predates the
model — LFM3 was released 2026-10-27 and its architecture did not
exist in that build. As deployed today, agentd's fallback runtime
could not serve LFM3 at all.

To get real numbers, a current llama.cpp was built ON the phone in a
throwaway Debian container (same pattern as the original build):
master commit 6cb3e672e8d10fd4feab1a048bf2967c40623133, Release,
static libs, targets llama-cli/llama-server/llama-bench, ~13 min,
exit 0. Binaries live in ~/agentic/models-lfm3/build/llama.cpp/build/bin/.
All LFM3 numbers below are from that build; the gemma baseline is
from the phone's own build — the build difference is unavoidable
(the old build cannot load the new arch) and is stated wherever the
comparison is used.

## Speed and memory (llama-bench, CPU, 4 threads, pp512 / tg128)

| model | runtime build | pp512 t/s | tg128 t/s | size | params |
|---|---|---|---|---|---|
| LFM3-1.7B Q4_K_M | llama.cpp 6cb3e67 (new) | 9.26 ± 0.07 | 5.92 ± 0.10 | 1.02 GiB | 1.73 B |
| gemma-3-1b-it Q4_0 (deployed fallback) | llama.cpp 0693dbf (phone's) | 18.79 ± 0.51 | 10.36 ± 0.18 | 680.82 MiB | 1.00 B |

LFM3 generates at roughly HALF the gemma fallback's speed on this
silicon — about 5.9 tokens/s, so a 60-token spoken reply is ~10 s of
pure generation before TTS even starts. Prompt processing of the
~1,300-token agent system prompt costs ~136 s at ~9 t/s on a cold
slot (measured in the scenario runs), which by itself disqualifies
the naive per-turn pattern for a voice loop.

RAM: podman stats sampled 934.8-938.4 MB for the LFM3 bench
container (gemma bench sampled 860.6 MB). During server runs the
same stats under-reported (166-264 MB, unreliable on this setup);
the system view is the steadier evidence: MemAvailable 4,550 MB
idle baseline -> 4,312 MB with the LFM3 server loaded -> 4,183 MB
mid-generation -> 4,431 MB after cleanup. Resident cost of the
loaded server is a few hundred MB — RAM is not the problem.
Speed and output behaviour are.

## Generation tests (llama-server, new build, --jinja, phone loopback)

### A — plain prompt (temp 0, cap 160)
User: "What is the capital of France? Answer in one sentence, then
name one museum there."
Result: finish=length, CONTENT EMPTY. All 160 generated tokens went
to the separate reasoning_content channel (6.62 t/s) and the budget
ran out mid-thought. Reasoning excerpt (verbatim):

"We need to answer: \"What is the capital of France? Answer in one
sentence, then name one museum there.\" So first part: The capital
of France is Paris. That's a straightforward fact. Then we need to
name one museum there. In Paris, famous museums include the Louvre,
Musée d'Orsay, Centre Pompidou, etc. We can pick any one..."

The model knew the answer instantly and never said it — the visible
channel got nothing.

### B — agent prompt, KEY TEST 1 shape (temp 0, cap 256)
System: brief agent instructions (reply briefly; end with an
ACTIONS: [...] line; confirm inferred destinations). User: "hey
phone open maps and give me directions to the airport".
Result: finish=length, CONTENT EMPTY again; 256 tokens of reasoning
at 5.96 t/s. The reasoning was heading somewhere reasonable (it
planned to ask which airport) but no answer was ever emitted.

### C — tool-call-shaped request (OpenAI tools parameter)
System: function-calling framing; tools=[app_launch(app,
destination)]; user: "Open maps and show me San Francisco."
Result: finish=tool_calls — a proper structured API tool call,
arguments exactly right (98 tokens at 5.95 t/s):

TOOL_CALLS: [{"type": "function", "function": {"name": "app_launch",
"arguments": "{\"app\":\"maps\",\"destination\":\"San Francisco\"}"},
"id": "bR8MxJ4HLxBLd6sgs2YlkJWjNnKAxGX5"}]

This is the one unambiguously strong result: when tools are offered
through the API, LFM3's native tool calling is clean and correct
on the first try.

## Scenario smoke — KT-001, KT-012, RT-002

Method, stated plainly: the Rust user-sim shim cannot be pointed at
a local server — its endpoint is a pinned constant
(tools/user-sim/src/main.rs: SHIM_API = "https://api.meta.ai/v1/
chat/completions", with a unit test asserting that URL) — so the
three scenarios were driven against the on-phone llama-server by a
small driver replicating the shim's contract byte-for-byte: same
system prompt (SHIM_SYSTEM + BRAINFIX v4, extracted from source),
same per-turn framing ([scenario ...] + WORLD CONTEXT block + raw
user text), same history handling, temp 0.3. The transcripts were
then scored by the REAL harness (user-sim --rescore against both
corpora). Raw transcripts: lfm3-report.json + journal.txt in
~/work/lfm3/ on the 3090.

Score: 0/3 — all three FAIL (intent class). The cause is
behavioural, not scorer vocabulary:

- 6 of 8 turns produced an EMPTY visible reply. Under the full
  agent prompt the model spends its entire budget in the reasoning
  channel: KT-001 turn 0 took 337 s (1,221 prompt tokens at
  8.99 t/s, then 1,024 generated tokens at 5.08 t/s — the cap)
  and delivered nothing the agent could speak or act on.
- KT-001 turn 1 was the one clean conversational answer, with the
  world-context facts exactly right, but no ACTIONS line:
  "It'll take about **55 minutes** via I-880 (no tolls on the fast
  route). Want me to start giving you directions now?" (actions: [])
- KT-012 turn 1 ("yes go ahead") produced the run's only
  structured act — in LFM3's native XML tool format, inside prose
  content, invisible to the ACTIONS contract (verbatim):
  <tool_call>
  <function=start_navigation>
  <parameter=destination>
  San Francisco International Airport (SFO)
  </parameter>
  </function>
  </tool_call>
- must_confirm was never violated — trivially: the model never
  navigated at all under the contract. RT-002 scored the extra
  reason "agent produced no reply and no actions".

### Two probes that isolate the cause (same KT-001 turn 0)
1. reasoning_budget: 0 in the request -> reasoning still 1,483
   chars, and content was ONLY the bare XML tool call:
   <tool_call><function=start_navigation><parameter=destination>
   SFO Airport</parameter></function></tool_call>
2. chat_template_kwargs enable_thinking: false -> reasoning fully
   off, and content became a coherent, context-accurate answer
   (SFO, 55 minutes, I-880, no tolls) — followed by the same XML
   <tool_call> block. Still no ACTIONS line in either probe.

So the failure is a format/config mismatch, not raw capability:
with thinking disabled the model answers well and uses the world
context correctly — but it speaks XML tool calls, never the
ACTIONS: [...] contract the phone agent parses, unless tools are
offered through the API (case C), where it is flawless.

## Tool-format observation (summary)

1. Native API tool calling (tools parameter): proper structured
   tool_calls JSON, correct arguments, first try. Excellent.
2. Free-form chat: emits LFM3's XML-flavoured
   <tool_call><function=...><parameter=...> blocks — the XML format
   the 2026-11-02 Ally research flagged as spike check #1. It never
   adopted the ACTIONS: [...] line format despite that format being
   an explicit system-prompt rule.
3. Default template/server behaviour splits chain-of-thought into
   reasoning_content and preserves it; under the deployed-pattern
   API the visible content channel starves — most turns returned
   empty content at the token cap.

## Storage and cleanliness

- Phone storage: 71.5 GB free before -> 70.1 GB free after.
  ~/agentic/models-lfm3/ totals 1.4 GB (GGUF 1.02 GiB + the
  llama.cpp build tree + prompts). The 3 GB-free guard was never
  approached; the build tree is removable scratch.
- Phone left clean: YES. Server/bench/build containers removed
  (only the pre-existing nord container remains, untouched), no
  llama or download processes, the three transient systemd units
  (download, driver, probe) all inactive, agentd never started or
  stopped by this test, agentd-main sha256 unchanged
  (f165f7063bbad9ec...).
- Observation for root, NOT caused by this test: agentd-main-keytest
  hashes to b2d3ee9ce926... now, vs 45a59b032564... recorded on
  2026-11-03 ~07:51 PT. Some other session changed that binary
  during the day; this test only ever read hashes in ~/agentic/bin.

## Verdict

No — LFM3-1.7B is not, on this evidence, a credible drop-in
replacement or upgrade for the gemma3-1b on-device fallback:
the deployed runtime cannot load it at all (architecture predates
the model), it generates at about half the fallback's speed on
this silicon (5.9 vs 10.4 t/s), its prompt processing makes the
agent system prompt cost ~136 s cold, and its default output
behaviour (reasoning-channel starvation plus XML tool calls
instead of the agent's ACTIONS contract) scored 0/3 on the
KEY TEST scenarios. Its genuine strength — clean, correct,
structured API tool calling — would only pay off after real
integration work (a newer llama.cpp in the deployment, thinking
disabled or budgeted in the serving config, and a tool bridge
that offers tools via the API instead of the ACTIONS contract).
That is a project, not a swap.
