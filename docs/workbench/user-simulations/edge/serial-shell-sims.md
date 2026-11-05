# Serial shell simulations — Track B stage-3 operator shell

Human-factors checklist for the stage-3 CDC-ACM serial shell
(OnePlus 6, Redox, connected from the Mac with
`screen /dev/cu.usbmodem* 115200`). Each session is scripted
the way a real operator actually types: typos, corrections,
interruptions, accidents. "Pass" is what the shell must do;
"Fail signs" are what a reviewer should call out on the spot.

Notation: `\r` = Return key, `\x7f` = Backspace/Delete key as
sent by macOS terminals, `\x03` = Ctrl-C, `\x01`/`\x05` =
Ctrl-A/Ctrl-E, `\x15` = Ctrl-U, `\x04` = Ctrl-D, literal text
is typed exactly as shown. Sessions assume the shell prompt
is the RAM-shell prompt shown at stage 3 (written `PROMPT>`
here; substitute the real prompt string when reviewing).

---

## SHELL-01 — First connect and orientation

Goal: prove the shell is alive and discoverable by a human
who has never seen it.

Keystrokes:
```
[connect USB, open screen]
\r
help\r
```

Pass:
- A banner or prompt appears without the operator having to
  guess a baud rate or send a magic wake-up sequence.
- An empty Return just re-displays the prompt (no error, no
  hang).
- `help` lists the commands that actually exist — every
  listed command runs, and no dangerous command is listed
  without being real.

Fail signs: silence on connect with no hint why; `help`
lists commands that return "unknown"; a stale prompt from a
previous session is presented as if fresh.

## SHELL-02 — Typo fixed with Backspace

Goal: ordinary line editing must work the way fingers
expect.

Keystrokes:
```
statsu\x7f\x7fus\r
```

(the operator typed `statsu`, backspaced twice, typed `us`,
producing `status`)

Pass:
- Each `\x7f` visibly erases one character (echo shows the
  correction, no `^?` garbage or doubled characters).
- The executed command is `status`, and its output is the
  normal status output.
- Backspacing at an empty line (try a few extra `\x7f`)
  does nothing harmful — no beeping loop, no prompt damage.

Fail signs: the shell executes `statsu` or a mangled hybrid;
erase only works visually but the buffer keeps the typos.

## SHELL-03 — Ctrl-C at an idle prompt

Goal: the most common reflex in a terminal must be boring.

Keystrokes:
```
\x03
\r
status\r
```

Pass:
- Ctrl-C at an empty prompt either does nothing or prints a
  fresh prompt line. It must not reboot, reset, exit the
  shell, or drop the USB connection.
- The shell is fully usable immediately afterwards
  (`status` runs normally).

Fail signs: Ctrl-C kills the shell, triggers the TTL reset,
or leaves the terminal echoing nothing.

## SHELL-04 — Ctrl-C during a long-running command

Goal: interrupt a command that is streaming output.

Keystrokes:
```
log\r
[wait ~2 seconds of output]
\x03
status\r
```

(If the shell has no streaming command, use its longest
output — e.g. `help` on a slow link or a memory dump —
and interrupt that.)

Pass:
- Output stops within about a second of Ctrl-C.
- A fresh prompt returns, any partial line of output is
  visibly cut off (not silently completed later).
- The next command runs cleanly; no leftover output from
  the interrupted command leaks into it.
- The device does not reset and the USB link stays up.

Fail signs: output runs to completion anyway; the prompt
never returns; the shell or device resets.

## SHELL-05 — Unknown command

Goal: mistakes must be cheap and informative.

Keystrokes:
```
frobnicate\r
stats\r
```

Pass:
- Each unknown command gets a short, plain error — the
  command echoed back, "unknown command" (or equivalent),
  and a pointer to `help`.
- No crash, no hang, no reset; the prompt returns
  immediately both times.
- A near-miss like `stats` does not get silently executed
  as `status` (no fuzzy auto-correction into a real
  command).

Fail signs: silent acceptance, a stack trace/panic on the
console, or the shell guessing and running something else.

## SHELL-06 — Accidental paste of a long line

Goal: survive the operator pasting a paragraph (e.g. a log
line or a URL with parameters) into the terminal.

Keystrokes:
```
[paste one single line of 400+ characters: a long URL plus
log text, no newline until the very end]
\r
status\r
```

Pass:
- The shell never executes a truncated prefix as if it
  were the whole command.
- Overlong input is handled in one of two acceptable ways:
  the full line is accepted and processed as one (unknown)
  command with a normal error, or the line is rejected
  with a "line too long" style message and discarded
  whole.
- No buffer corruption: the following `status` command
  works normally, and no characters from the pasted line
  reappear or execute later.

Fail signs: the first N characters run as a command; the
shell hangs or resets; fragments of the paste execute on
later Returns.

## SHELL-07 — Disconnect and reconnect mid-command

Goal: the USB cable gets bumped / `screen` is closed while
a command is running.

Keystrokes:
```
log\r
[while output is streaming: close screen / unplug USB]
[reconnect, reopen screen]
\r
status\r
```

Pass:
- On reconnect the operator gets a live prompt — either
  the original session continues sanely (with the
  interrupted command clearly finished or clearly
  cancelled) or a fresh session starts with a visible
  banner/prompt.
- The interrupted command is never executed a second time
  as a side effect of reconnecting.
- `status` after reconnect tells the truth about uptime /
  session state (a reset, if one happened, is visible —
  e.g. uptime restarted — not disguised).

Fail signs: a dead terminal that echoes nothing; the
streaming command restarts from the beginning on its own;
the shell claims state (uptime, history) that a reset
should have cleared.

## SHELL-08 — Empty lines, whitespace, and line endings

Goal: sloppy input hygiene.

Keystrokes:
```
\r
   \r
  status  \r
status\r\n
```

(last line sent with CRLF, as some terminals do)

Pass:
- Empty and whitespace-only lines just re-prompt.
- Leading/trailing spaces do not break a real command.
- CRLF is treated as one Enter, not as a command plus a
  mystery empty command with an error.

Fail signs: "unknown command" errors for blank input; the
`\r\n` line producing a phantom second command or error.

## SHELL-09 — Escape sequences and stray control keys

Goal: arrow keys and friends must not corrupt the line.

Keystrokes:
```
st\x1b[D\x1b[Catus\r
st\x01xx\x05atus\r
```

(arrow-left/right escape sequences mid-word; then a line
with Ctrl-A, junk, Ctrl-E)

Pass:
- Escape sequences are either handled (real cursor
  movement) or safely ignored — in neither case do the raw
  bytes (`[D`, `[C`) end up in the command buffer.
- Control keys the shell does not support are ignored
  without side effects; the line that finally executes is
  a coherent command or a normal "unknown command" error,
  never a corrupted hybrid that runs something unintended.

Fail signs: `st[D[Catus` style garbage reaching the parser;
a control key resetting the shell or the device.

## SHELL-10 — Aborted dangerous command

Goal: a destructive command must not fire from a partial
line.

Keystrokes:
```
rese\x03
\r
status\r
```

(the operator starts typing `reset`, thinks better of it,
and Ctrl-C's before pressing Return)

Pass:
- Nothing resets. The partial line is discarded.
- The following empty Return just re-prompts, and `status`
  shows the device still in the same session (uptime
  continuous with earlier sessions).
- Relatedly: `reset` typed in full but followed by
  anything the shell defines as a cancel must not reset —
  and if `reset` requires confirmation or specific syntax,
  `help` says so.

Fail signs: any reset, reboot, or session drop traceable
to the aborted line.

## SHELL-11 — Line-kill keys and editing recovery

Goal: recover from a badly mangled line without sending it.

Keystrokes:
```
status xyzzy garbage --nonsense\x15
status\r
```

(Ctrl-U `\x15` after the junk, intending to clear the line;
if the shell does not implement Ctrl-U, the operator falls
back to holding Backspace.)

Pass:
- One of two acceptable behaviours: Ctrl-U clears the
  whole line (visibly), or Ctrl-U is ignored and the
  operator can still backspace the line away — either way
  the garbage line is never executed.
- The clean `status` that follows runs normally.
- If the shell does document line-editing keys (in `help`
  or a banner), they behave as documented; undocumented
  keys never do something surprising like executing the
  line.

Fail signs: Ctrl-U submits the line; editing keys delete
the prompt or corrupt the display so the operator cannot
tell what will execute.

## SHELL-12 — Paste of several commands at once

Goal: the operator pastes a small runbook block.

Keystrokes:
```
[paste as one block:]
help
status
frobnicate
status
```

Pass:
- Commands run one at a time, in order, each with its own
  output and prompt; the unknown third command errors
  normally and does not stop or corrupt the fourth.
- No interleaved or merged output that makes it unclear
  which output belongs to which command.
- Final state: a clean prompt, shell responsive.

Fail signs: pasted lines merging into one mega-command;
commands after the error never running; output from two
commands interleaved character-by-character.

---

## Reviewer notes

- Run SHELL-01 → SHELL-05 first on tonight's stage-3 boot;
  they are the minimum "a human can drive this" bar.
  SHELL-06 → SHELL-12 can follow on the same boot if time
  allows, or on the next shell iteration.
- Record for each session: pass/fail, the literal prompt
  string, and any fail sign observed — the fix list for the
  shell comes straight from these notes.
- Anything in these sessions that resets the device outside
  SHELL-10's explicit subject is an automatic stop: note it
  and end the shell portion of the run.
