# KEY TEST 1 — wake word ("hey phone") for the phone

**Status: WORKING (file-mode proven 2026-11-03 ~07:49 PT; live-mic
proof is the owner's tonight, commands below).** This closes gap 1 of
KEY-TEST-1 ("no wake word exists") with a v1 listener built only
from what the phone already had. Deployed additively to
`~/agentic/wakeword/` on the phone; no deployed Track A file was
modified, and nothing is left running when it is stopped.

**Update 2026-11-03 (evening): the wake phrase is now "hey phone"**
(operator's choice; `WAKE_RE = /\bhey[\s,]+phone\b/i`), and two
defects found in the first live session are fixed and proven on the
phone (see "Proof — the 2026-11-03 fixes" below): the listener now
runs as a **systemd user unit** so it survives the SSH session that
started it, and a wake match whose utterance already contains the
request **fires on that utterance's own audio** (one-breath fire)
instead of re-capturing a live mic window.

## Design chosen: 2 — persistent Moonshine + energy VAD

```
mic (arecord 16 kHz raw)
  -> wakeword.py (host, python3 stdlib): energy VAD segments speech
  -> inbox/ dir  ->  ww_engine.py in ONE long-lived podman container
                     (Moonshine transcriber loaded ONCE)
  -> outbox/ dir -> transcript;  /\bhey[\s,]+phone\b/i  = FIRE
  -> exactly one voiced turn (run sheet Step 3 command shape):
     on the utterance's own audio when a request follows the phrase,
     otherwise on a fresh 8 s live capture
  -> back to listening
```

Why not design 1 (openWakeWord "hey jarvis")? Probed honestly:
the wheels ARE obtainable — `pip download` inside the voice image
(with `--network host`; plain podman networking fails on this
phone with a netavark/nft error) fetched
`onnxruntime-1.31.1-cp311-aarch64` and `openwakeword-0.6.0`. But
nothing of that stack exists on the phone today: no runtime
installed, no model integrated, no inference front-end — it would
be a second, unproven speech stack to build and tune tonight.
Moonshine is already staged on the phone and proven (a real voiced
turn ran through it on 2026-11-02), and its transcriber loads in
~0.2–0.3 s and stays resident, which removes the per-chunk
container-start cost that made a naive loop impossible. Design 1
remains a valid later swap behind the same inbox/outbox contract.

Why the arch default is `tiny`: wake detection only needs a short
phrase reliably; the fired voiced turn transcribes with the
deployed `small` STT anyway — on the matched utterance itself when
the request was spoken in the same breath, or on a fresh capture
when only the phrase was said. Measured on the test sentence:
tiny transcribes it perfectly in ~1.2 s vs small ~3.0 s.
`WW_ARCH=small ./wakeword.sh start` switches back.

## Files (phone, all additive)

| Path | Role |
|---|---|
| `~/agentic/wakeword/wakeword.sh` | start / stop / status / test-file; start launches the listener as systemd user unit `wakeword-listener` and records its MainPID in `listener.pid` |
| `~/agentic/wakeword/wakeword.py` | host listener: VAD, engine client, fire |
| `~/agentic/wakeword/ww_engine.py` | in-container persistent transcriber |
| `~/agentic/wakeword/tests/*.wav` | the 3 proof files (Piper-synthesised) |
| `~/agentic/wakeword/heard/` | every utterance the listener transcribed |
| `~/agentic/wakeword/listener.log` | listener log |

Canonical copies live in this repo under
`docs/workbench/user-simulations/key-tests/wakeword/`.
One pre-existing-path file may be created by `start`:
`~/agentic/voiced-tonight.conf`, made with exactly the run sheet's
Step 2 sed, only if absent — so the fired turn is audible. The
deployed `voiced.conf` is never written.

## Start / stop (tonight)

```sh
ssh oneplus '~/agentic/wakeword/wakeword.sh start'   # engine + listener
# ... first do run sheet Step 1 (agentd) or the fired turn has
#     no agent to talk to; order between the two does not matter
ssh oneplus '~/agentic/wakeword/wakeword.sh status'
ssh oneplus '~/agentic/wakeword/wakeword.sh stop'    # everything off
```

`start` launches the listener via `systemd-run --user
--unit=wakeword-listener`, so it keeps listening after the SSH
session that started it closes (the original nohup launch died
with its session). `stop` stops the unit and also honours the
pidfile, so a listener started by hand is still stopped cleanly.

Then just say it, phone at talking distance:

> **"Hey phone, open maps and give me directions to San Francisco."**

The listener logs the transcript and `WAKE WORD MATCH`, stops its
own capture, and runs the Step 3 voiced turn, then resumes
listening. Speak the whole sentence in one breath: because the
utterance already contains the request, the turn runs on that very
audio (`voiced --wav <heard file>`) — there is no second capture
window and nothing to repeat. If you say only "hey phone", the
turn instead opens the original 8 s live capture: speak the
request when it starts.
`WW_ONCE=1` in the environment makes it exit after one fired turn.

## Proof — file mode (2026-11-03, this build, this phone)

Test WAVs synthesised with the phone's own Piper (`tts-ptts.sh`),
run through the full detector path (`wakeword.py --file`,
detect-only; engine = Moonshine, arch `small` unless noted).
Current fixture set (regenerated for the "hey phone" phrase;
re-run 2026-11-03 22:54 UTC, arch `tiny`):

| File | Spoken text | Transcript | Verdict |
|---|---|---|---|
| `tests/pos.wav` | "Hey phone, open maps and give me directions to San Francisco." | identical | **FIRE** |
| `tests/neg-oldword.wav` | "Jarvis, open maps and give me directions to San Francisco." (the pre-2026-11-03 positive) | identical | **NO-FIRE** |
| `tests/neg-nowake.wav` | "Open maps and give me directions to San Francisco." | identical | **NO-FIRE** |
| `tests/neg-nearmiss.wav` | "Jarvison, open maps and give me directions to San Francisco." | "Jarvison opened maps and give me directions to San Francisco." | **NO-FIRE** |

Numbers: model load 308 ms (small) / 333 ms (tiny); engine ready
~2 s after container start. Transcribe of the ~4.3 s sentence:
3020 ms (small) / 1207 ms (tiny). The near-miss passes because the
match is a word-boundary regex, not a substring — and the old wake
word no longer fires at all (`neg-oldword.wav`).

Mic-mode smoke (same day): `wakeword.sh start` in a quiet room,
12 s — listener up, no false fire, no crash; `stop` returned the
phone to clean (verified: no listener process, no engine
container, no arecord). Playing audio at the phone in an empty
room would prove little, so the live-mic proof is tonight's, with
the commands above.

## Proof — the 2026-11-03 fixes (persistence + one-breath)

**Persistence.** Started via the fixed script in one SSH session
(22:54:36 UTC: unit `wakeword-listener`, MainPID 222237, pidfile
records 222237); that session then closed. In a fresh session
3 min 32 s later (22:58:12 UTC) the unit was still `active` with
the same MainPID and the engine running — the pre-fix nohup
listener died in exactly this gap.

**One-breath fire, acoustic end-to-end.** With agentd-keytest up
and the listener running from the fixed script, `tests/pos.wav`
was played through the phone's own speaker (`paplay`) at
23:03:32 UTC. From the logs/traces:

```
23:03:38 heard (1909 ms ...): 'Hey phone, open maps, and give me
         directions to San Francisco.'
23:03:38 WAKE WORD MATCH
23:03:38 one-breath fire: a request follows the wake phrase,
         running the turn on the utterance itself
         ($HOME/agentic/wakeword/heard/utt-1793747016-1.wav)
23:03:38 FIRE - running voiced turn: voiced --wav
         .../heard/utt-1793747016-1.wav --config voiced-tonight.conf
         --journal .../wakeword-fire.trace
         --out .../wakeword-answer.wav --timeout 150
wakeword-fire.trace: stt_final "Hey phone, open maps and give me
         directions to San Francisco." (conf 0.8); playback:paplay,
         playback finished
agentd trace: execute_tool app_launch span_end status ok —
         {"app":"maps","launched":true,
          "uri":"geo:0,0?q=San%20Francisco","via":"gio open"}
voiced: heard "...San Francisco." -> answer "Maps is open with
         San Francisco on the map ..." -> wakeword-answer.wav
         (241,964 bytes)
23:03:57 voiced turn exit=0 in 19.4s; resuming listening
```

Honest footnote: the first attempt at this proof (22:58:19 UTC)
matched and fired correctly on the utterance — its STT even
completed — but an external `systemctl --user stop` of the unit
landed at 22:58:21, mid-turn, and killed the turn with the unit
(the fired turn is a child of the listener; see Limits). The
re-run above is the same test, uninterrupted.

## Limits — honestly

- **Latency:** from end of your sentence to the match decision is
  ~1.2 s (tiny) plus VAD hangover 0.6 s. It feels like a pause,
  not a snap. The fired voiced turn then pays Track A's normal
  one-shot costs (throwaway STT/TTS containers; plus an 8 s live
  capture only when the utterance was the bare phrase).
- **VAD is energy-only.** TV, music, or a loud room will segment
  speech that is not yours; each false segment costs one
  transcription, and a fire still requires the phrase "hey phone"
  in a transcript. Other people saying "hey phone" WILL fire it —
  there is no speaker verification.
- **Cost while listening:** the engine container measured
  ~8% CPU and ~108 MB RAM (podman stats, smoke window); the model
  stays resident — that is the battery trade for a ~2 s wake. The
  host listener itself is negligible (100 ms RMS frames). Drain
  over a full evening was not measured.
- **One mic, one owner:** the listener releases the mic while a
  phrase-only fired turn captures, and re-opens it after (a
  one-breath turn does not capture at all). If another app holds
  the mic, arecord fails and the listener restarts its capture
  loop (watch `listener.log`).
- **The fired turn belongs to the listener unit.** It runs as the
  listener's child, so stopping the listener (or its unit) while a
  turn is in flight kills the turn too — observed live on
  2026-11-03, when an external unit stop at 22:58:21 UTC landed
  2 s into a fired turn and ended it mid-STT. Let a fired turn
  finish before stopping.
- **agentd is a separate step:** the listener fires the voiced
  turn regardless; without agentd running (run sheet Step 1) the
  turn fails and the failure is in the log, not silent.

## CV1 — conversation session (follow-up window + agentd session state)

Built 2026-11-03 (workstream CV1; operator decision: the window is
about 10 seconds, on-device, with the listening state visible in
kimono for the whole window).

**Window mechanics (listener).** A fired turn that succeeds opens a
session (`sess-<epoch>-<n>`, minted by the listener) and, when the
answer has finished, a follow-up window of `WW_FOLLOWUP_S` seconds
(default 10). Inside the window the mic is hot and the listener's
Moonshine engine transcribes; the first utterance with a real
transcript fires the next voiced turn WITHOUT the wake word
(`voiced --wav <utterance> --session <id> --follow-up`). An utterance
that starts inside the window may finish past it (bounded grace);
an empty transcript neither fires nor extends the window. Each
follow-up answer re-opens the window; a fully silent window ends
the session. The window exists only immediately after an answer in
an active session — never ambient.

**Truthful indicator.** The listener reports every window
transition to agentd (`POST /follow-up {"session","state"}`),
posted before capture starts on open and after it stops on close.
agentd journals a `follow_up_window` record, and the kimono mic
chip renders "still listening · follow-up window" from exactly that
record (render.mjs folds it into `view.followUp`; nothing else may
set it). While a fired turn runs, the window is reported closed —
the follow-up mic is not hot then, and the page does not claim it
is. Headless proof: `apps/kimono-ui/test/followup_test.mjs`.

**Session state (agentd).** `POST /request` accepts optional
`session` and `follow_up` fields (a sessionless request is the old
wire exactly). agentd's `SessionStore` holds the session's turns,
entities (a successful `app_launch` sets `destination`), and
pending slots (a broker-blocked label; an answer ending in a
question). A turn reaches the brain with the earlier turns replayed
as user/assistant history plus the entities as a background note
marked already-acted-on. The system prompt — BRAINFIX v4 included —
is built exactly as for a sessionless request; context is data,
never instruction. Every run journals a `session_context` record
(session, prior turns, entities, pending, follow-up mark, fresh or
continuing). Sessions expire whole after 90 s without a turn; a new
session id inherits nothing. Host proof:
`apps/agentd/tests/session_run.rs` (carry-over, expiry, isolation,
and the brain-boundary payload asserted message by message).

**Documented limit (KT2 C6, not solved).** Inside a hot window any
voice is treated as the session's speaker — there is no speaker ID.
A bystander (or a TV) speaking inside the window becomes the next
turn. BRAINFIX's owner-confirmation rule still governs inferred
destinations, and the window is short and visibly indicated, but
the risk is real and recorded here rather than engineered away.
