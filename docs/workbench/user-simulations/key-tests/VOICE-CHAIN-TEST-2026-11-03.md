# VOICE-CHAIN TEST — synthetic end-to-end check of the phone voice chain

OnePlus 6 (`user@MESH_HOST`), 2026-11-03 ~22:37 UTC.
Test artifacts only; no deployed file or config changed.
Live-mic diagnosis (capture/VAD/gain) is Remy's parallel lane.

Sentence: "Hey phone, open maps and give me directions to San Francisco"

## 1. Synthesis (Piper path)

`tts-ptts.sh $HOME/agentic/tmp/voice-chain-test.wav` — exit 0.
192,044 bytes, 24 kHz mono, 4.0 s, peak 15695 (rtf 0.514).

## 2. Wakeword detector (`wakeword.sh test-file`)

Transcript: "Hey phone, open maps and give me directions to San Francisco."
`WAKE WORD MATCH` -> `RESULT: FIRE`. The rebuilt HEY PHONE
detector (`WAKE_RE = \bhey[\s,]+phone\b`) fires on clean input.

## 3. Voiced turn (`voiced --wav ... --config voiced-tonight.conf`)

- `stt_final`: "Hey phone, open maps and give me directions to San Francisco." (conf 0.8)
- agent answer: "Maps is open on the phone showing San Francisco — tap the route button there for turn-by-turn directions."
- answer WAV: YES — `$HOME/agentic/tmp/voice-chain-answer.wav`, 272,684 bytes, 24 kHz, 5.68 s, played via paplay. Turn 18.1 s, exit 0.

## Verdict

Synthetic chain is fully green: TTS -> detector (FIRE) -> STT ->
agentd (`agentd-main-keytest`, health ok) -> TTS answer. Tonight's
live failure (no transcripts since the 22:25 UTC HEY PHONE restart,
no `keytest1.trace`) is therefore in the live capture path, not in
the detector phrase, STT, agent, or TTS.
