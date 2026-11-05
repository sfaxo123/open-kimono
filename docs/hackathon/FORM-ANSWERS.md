# Form answers (paste-ready)

## Project name

Open Kimono: an agentic phone OS, kernel to voice

## Tagline (<=100 chars)

An agentic phone OS, kernel to voice: it does what you say and shows its work.

## Short description (<=300 chars)

Open Kimono puts an agent daemon, Lego service blocks, an open trace UI, and a wake-word voice loop on a OnePlus 6, above a Redox OS port booted RAM-only. Ask for a water reminder and it installs a real timer, fires notices, stops on command, with every step journaled and visible.

## Full description (<=500 words)

Open Kimono is an agentic phone OS, built from the kernel up to the voice that answers you.

The problem: today's phone assistants are black boxes. You speak, something happens somewhere, and you cannot see what it did or why. And on-device agents need a real OS story — boot, drivers, power, mic discipline — not just a chatbot stapled to a launcher.

What it does: an agent daemon runs on the phone itself. Type or say "remind me every 2 minutes to drink water" and it installs a real timer service, fires notices, and stops on command. Every run is journaled and streamed to an open UI that shows runs, tool calls, power changes, and grants instead of hiding them. A voice loop with the wake phrase "hey phone" handles mic ownership, speech-to-text, the agent turn, and text-to-speech, with a quiet mode that answers by WAV bytes without playing sound or opening the live mic. Power advice comes only from measured battery drain, and radios stay dry-run.

Underneath, a Redox microkernel port runs on the OnePlus 6 hardware, booted RAM-only so nothing on the device is ever flashed or harmed. Rust drivers and protocol crates sit above the kernel, and the agentic layer runs as portable services designed to move onto the new OS later.

Commit safety is the core idea: installing a self-firing block asks for confirmation first, with an exact label, and a Confirm button on the phone UI sends it. The mic has a single on/off source of truth, and mic-off really blocks listening everywhere.

Verified so far: the water-reminder demo runs end to end on the phone with a full trace; the commit gate, routines-as-services, power manager, and mic truthfulness each have their own tests with negative cases; the wake-word listener runs on the phone; the candidate phone brain held conversations across 39 turns in a 16-scenario key test; the Redox kernel RAM-boots on the real phone and resets it back to postmarketOS on its own timer (5 of 5 and 3 of 3 unattended cycles); it boots to a serial rescue shell in emulation; and 1,327 host tests pass across 31 suites.

Next: run the queued 90-second phone demo, take real on-device benchmark measurements, wire anticipation to a live context source, build the waveform widget, and move the Muse Spark brain run from the host onto the phone.

## Meta models / Meta tech used

Muse Spark (muse-spark-1.3-contributor) as autonomous coding contributors: isolated workers, one packet and one commit each, budget-capped. Muse Wonder (muse-wonder-1.0) as independent reviewer of every new diff, and as the evaluated phone-brain candidate in the 16-scenario key test. Muse Spark (muse-spark-1.3) is also the agent's brain: through the Meta Model API it writes the timer service, waits at the commit gate, installs after the user confirms, and stops the service on command. All 44 model spans in the demo trace are Muse.

## Tech stack

Rust throughout; Redox OS microkernel; RAM-only fastboot boot; systemd user units on the device; server-sent events UI; Wyoming voice protocol; speech-to-text and text-to-speech engines as config; sysfs battery sensing; JSON trace journals.

## What we are proudest of

The honesty discipline: every claim names its proof, emulator results are labelled emulator, and open items stay open. The commit gate is real — a self-firing service cannot install itself without the user's exact confirmation. And the whole stack, kernel to voice, is one coherent build, not a demo app on someone else's OS.

## Challenges

The phone kept leaving the network mid-demo, so every run had to be resumable and trace-backed. Voice needed mic discipline with no second state machine and no invented waveform. Power advice had to come from the one measurable signal, battery drain, without inventing per-component numbers. And the key test showed the brain candidate still guesses destinations without confirming — the exact failure to fix next.

## Next steps

Run the queued phone demo and bank its trace; measure on-device benchmarks and pin the model; wire anticipation and pattern sync to real sources; build the waveform widget; run the Muse Spark brain from the phone itself (host-proven today); and keep marching the new OS boot until the agentic layer runs on it directly.

## Repo:

https://github.com/OWNER/open-kimono
