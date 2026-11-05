# KEY TEST 1 — on-phone run sheet (tonight, ≤ 15 minutes)

**The test (operator, 2026-11-03):** hold a conversation with the phone —
wake word, then one simple real request spoken aloud:
**"open maps and give me directions."**

Run this at home, phone in hand, Mac nearby for the `ssh oneplus`
commands. Everything below was verified read-only against the real
phone on 2026-11-03 ~07:40 PT; where a leg cannot work tonight, the
sheet says so and still scores the legs that can.

**Tonight's headline, honestly:** the voice conversation leg is fully
runnable (mic → on-device speech-to-text → agent → spoken answer).
Two legs cannot pass as specified: there is **no wake word** (nothing
listens until you start a capture — the sheet uses push-to-talk), and
**the agent cannot open Maps** (agentd has no app-launch tool; Maps
itself is installed and one command away — you run that command, the
sheet scores it as assisted, not agent-done).

---

## Ground truth per leg (verified 2026-11-03, phone `oneplus-enchilada`)

| Leg | State | Evidence |
|---|---|---|
| Phone up, reachable | ✅ | pmOS up 1 day+, Meshnet SSH answers, battery/mesh fine |
| agentd (the agent) | Present, **not running** — you start it in Step 1 | `~/agentic/bin/agentd-main` sha256 `f165f7063bbad9ec` (the current build, with the confirm step); no agentd process and no systemd unit exists — it is only ever started manually |
| agentd config for tonight | ✅ | `~/agentic/agentd-kimono.toml`: listen `127.0.0.1:8770`, kimono page in `static_dir`, brain = gateway model `5090` via the 3090 LiteLLM (`MESH_HOST:4000`), fallback local `gemma3-1b` |
| voiced (voice turn) | Present, one-shot CLI | `~/agentic/bin/voiced` sha256 `f2b6fc0e034122d8`; runs ONE turn per invocation (`--mic … --seconds 8`) and exits; config `~/agentic/voiced.conf` |
| Wake word | ❌ **Absent** | No wake-word/hotword code anywhere in Track A (`apps/`) or on the phone; voiced only captures when invoked. Substitute tonight: push-to-talk (Step 3) |
| Mic capture | ✅ hardware + path present | sdm845 sound card with capture PCMs; PulseAudio running, default source `…HiFi__Mic1__source`; `arecord` installed (voiced's documented capture command) |
| Speech → text | ✅ on-device | voiced.conf STT = `~/agentic/bin/stt-moonshine.sh` → Moonshine (small) in a throwaway podman container; image `localhost/voice-snapshot:20261024` present (5.68 GB). Adapters verified standalone on the phone; one real voiced turn reached agentd on 2026-11-02. NOT the Ally Parakeet chain — that is the Roomba gateway path, not the phone's |
| Text → speech | ✅ on-device, but muted by config | TTS = `~/agentic/bin/tts-ptts.sh` (Piper/Phonon, same container image). Deployed conf has `play = none`, `quiet = true`, so tonight you first make a tonight-conf with `play = paplay` (Step 2); `paplay` exists on the phone. With the stock conf the turn still succeeds — the answer is written to a WAV and never played (`playback:none` in the journal) |
| Maps app | ✅ installed | **GNOME Maps 51.1** (`gnome-maps-51.1-r0`, `org.gnome.Maps`). No Pure Maps / OsmAnd / Organic Maps |
| Maps opens on a destination | ✅ by command — ❌ by the agent | GNOME Maps is the registered `geo:` handler (`x-scheme-handler/geo=org.gnome.Maps.desktop` in mimeinfo.cache; desktop `Exec=gapplication launch org.gnome.Maps %U`). Exact invocation: `gio open "geo:0,0?q=<destination>"`. But agentd's tool list is only `phone_clock`, `phone_status`, `power_status`, `power_plan`, `service_create/describe/list/stop` — **no launch/open tool exists**, so the agent cannot do this step |
| Directions content | ⚠️ model prose only | agentd has no routing tool or live map data; any directions it speaks are the model's own words — plausible route text, not real routing. GNOME Maps itself does real routing in its UI once open (one tap on its route button) |

---

## The run (target ≤ 15 min)

### Step 0 — set up (1 min)
Phone unlocked, volume up, in your hand. Mac terminal open.
Say nothing yet — nothing is listening (no wake word; see gaps).

### Step 1 — start the agent (2 min)
```sh
ssh oneplus 'export XDG_RUNTIME_DIR=/run/user/$(id -u); systemd-run --user --unit=agentd-keytest --collect ~/agentic/bin/agentd-main --config ~/agentic/agentd-kimono.toml --trace-out ~/agentic/out/keytest1-agentd.trace'
ssh oneplus 'curl -s -m 5 http://127.0.0.1:8770/health'
```
**Success:** the health call returns a JSON reply (not an error /
empty). If empty: wait 10 s and retry once; still empty → write
**F4** and stop the unit (Step 6), test over.

### Step 2 — tonight's voice conf (1 min)
```sh
ssh oneplus 'sed "s/^play = none/play = paplay/; s/^quiet = true/quiet = false/" ~/agentic/voiced.conf > ~/agentic/voiced-tonight.conf && grep -E "^(play|quiet)" ~/agentic/voiced-tonight.conf'
```
**Success:** prints `play = paplay` and `quiet = false`. (The deployed
conf stays untouched; this copy is tonight-only.)

### Step 3 — the key-test turn (≤ 4 min)
Run this, then **speak immediately** — capture starts at once and
lasts 8 seconds. Hold the phone at normal talking distance:
```sh
ssh oneplus '~/agentic/bin/voiced --mic "arecord -q -f S16_LE -r 16000 -c 1 -t raw" --seconds 8 --config ~/agentic/voiced-tonight.conf --journal ~/agentic/out/keytest1.trace --out ~/agentic/out/keytest1-answer.wav --timeout 150'
```
Say, exactly:
> **"Jarvis, open maps and give me directions to San Francisco."**

("Jarvis" is decorative tonight — nothing listens for it; it rides
along in the transcript. If the transcript shows the agent stumbled
over it, retry once without it. Swap the destination for anywhere
you prefer.)

**Success:** within ~1–3 min (container STT/TTS is slow) the phone
**speaks an answer aloud** that contains directions to San Francisco,
and voiced exits 0.
**Partial success:** no sound, but the command exits 0 — go to Step 4
checks; the answer WAV may still be perfect (verdict P2 below).

### Step 4 — check what it heard and said (2 min)
```sh
ssh oneplus 'grep -E "stt_final|text_cleaned|playback" ~/agentic/out/keytest1.trace | tail -5'
ssh oneplus 'ls -la ~/agentic/out/keytest1-answer.wav && paplay ~/agentic/out/keytest1-answer.wav'
```
**Success:** `stt_final` shows roughly the sentence you said;
playing the WAV by hand gives the same answer you heard (or the one
you *should* have heard in Step 3). Write down the transcript
**word for word** if it differs from what you said → **F3**.

### Step 5 — maps (2 min, assisted — the agent cannot do this)
```sh
ssh oneplus 'export XDG_RUNTIME_DIR=/run/user/$(id -u); gio open "geo:0,0?q=San%20Francisco"'
```
**Success:** GNOME Maps opens on the phone showing San Francisco;
tap its route/directions button for real turn-by-turn.
**If the command errors** (session-bus quirks over SSH): tap the
Maps icon yourself, search `San Francisco`, tap Directions — and
write **F8-cmd** next to the verdict. Either way the maps *app* leg
is proven; the *agent-opens-it* leg is a known gap, not a surprise.

### Step 6 — stop the agent, fill the verdict (1 min)
```sh
ssh oneplus 'export XDG_RUNTIME_DIR=/run/user/$(id -u); systemctl --user stop agentd-keytest'
```
Keep `~/agentic/out/keytest1.trace` and `keytest1-answer.wav` —
they are the evidence for whatever verdict you write.

---

## If it fails — write the code + one line

| Code | Failure | What to write down |
|---|---|---|
| F1 | voiced never captured (command error before listening) | the exact error line |
| F2 | Capture ran but heard silence/garbage | room noise? how far was the phone? (`stt` exit 3 = silence) |
| F3 | STT wrote different words than you said | the `stt_final` text, verbatim |
| F4 | agentd unreachable (Step 1 health empty twice) | "health empty" — do not debug tonight |
| F5 | Agent answered a different request / nonsense | the answer text, one line |
| F6 | Agent asked for confirmation instead of answering | the exact question it asked (a plain directions ask should need none) |
| F7 | Answer existed as WAV but no sound in Step 3 | journal says `playback:…` what? Step 4 manual `paplay` result |
| F8 | Maps did not open, or opened on the wrong place | which: `F8-cmd` (gio failed) / `F8-place` (wrong result) |
| F9 | Directions wrong or invented | the road/step it got wrong |

Retry budget tonight: **one** retry of Step 3 (quieter room, phone
closer). More than that is debugging, not a key test.

**Plan B if live capture fails (F1/F2):** record first, then feed
the file — same STT, same agent:
```sh
ssh oneplus 'arecord -q -f S16_LE -r 16000 -c 1 -d 8 ~/agentic/out/keytest1-q.wav'
ssh oneplus '~/agentic/bin/voiced --wav ~/agentic/out/keytest1-q.wav --config ~/agentic/voiced-tonight.conf --journal ~/agentic/out/keytest1.trace --out ~/agentic/out/keytest1-answer.wav --timeout 150'
```
**Plan C if voice fails entirely:** open `http://127.0.0.1:8770/` in
the phone browser (kimono page, served by the agentd you started)
and *type* the same sentence. That still scores the conversation
and directions legs — only the voice legs fail.

---

## Honest gaps — what cannot work tonight, and the smallest fix

1. **No wake word.** Nothing on the phone listens continuously;
   voiced is one-shot per invocation. *Smallest fix:* a tiny
   always-on listener (openWakeWord-class model on the mic stream)
   whose only job is to fire the Step 3 command on the word — the
   rest of the chain already works push-to-talk.
2. **The agent cannot open Maps (or any app).** agentd has no
   launch tool; Step 5 is you running `gio open`, not the agent
   acting. *Smallest fix:* one agentd tool, e.g. `maps_open` /
   `app_launch`, that runs the proven `gio open "geo:0,0?q=…"`
   invocation behind the existing broker/confirm rules.
3. **Directions are model prose, not routing.** No tool gives
   agentd live route data, so spoken directions can be invented.
   *Smallest fix:* the same `maps_open` tool returning the place
   it opened (agent narrates from the opened route), or a routing
   API tool — until then, treat Step 5's in-app route as the real
   directions and the spoken ones as a summary.
4. **Voice is slow and one-shot.** Each turn pays two container
   starts (STT timeout budget 180 s, TTS 240 s); a conversation
   means re-running Step 3 per sentence. *Smallest fix:* keep the
   engine containers warm, or a Wyoming server on the phone — a
   performance fix, not a correctness one.
5. **agentd is not a service.** Nothing starts it at boot; Step 1
   is manual every time. *Smallest fix:* one systemd user unit
   (the binary + kimono config above) — deliberately not done
   yet, so tonight you start and stop it by hand.

---

## Verdict box (fill in tonight)

```
Date/time: ____________   Destination used: ____________
Wake word leg:        n/a tonight (push-to-talk used) 
Heard me:             stt_final text: ________________________________
Agent answered:       Y / N    Directions sensible:  Y / N / F9
Answer spoken aloud:  Y / N (if N: paplay by hand worked? Y / N)
Maps opened:          Y (assisted, Step 5) / N    Failure codes: ______
VERDICT:  [ ] P1 conversation pass (voice in → directions → voice out)
          [ ] P2 partial — answer made but not heard live (F7)
          [ ] P3 partial — typed fallback used (Plan C)
          [ ] FAIL — codes: ______
FULL key-test pass (agent opens Maps itself): NOT ACHIEVABLE tonight — gap 2.
```
