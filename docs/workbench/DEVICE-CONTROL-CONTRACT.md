# Device control framing and remaining contracts

Partial0.2.1, 2026-10-27 6:40 PM PT. This is a proposed new host envelope,
not an existing Redox/USB protocol or tracejournal compatibility claim.
No live device transport or CLI command is implemented here.

## Implemented envelope v1

`crates/device-control` is dependency-free, no_std and forbids unsafe code.
Private Frame fields are constructed by validating a complete byte buffer;
the payload remains borrowed. Encoding validates before writing into the
caller-provided buffer, including its capacity. A caller may tighten the
16MiB payload limit; a looser policy is rejected, not clamped.

| Byte range | Contract |
|---|---|
|0..4|ASCII DCTL|
|4..6|LE u16 version1|
|6|One u8 operation: Logs1, Screenshot2, Command3, Input4|
|7|One u8 reserved, exactly0|
|8..16|LE u64 nonzero correlation ID|
|16..32|16byte boot identifier, not all zero|
|32..36|LE u32 payload length, at most16MiB and caller policy|
|36..end|Exactly the declared payload, no trailing bytes|

Nonzero IDs are structural validity only, not authenticated identity, uniqueness
or replay protection. The parser accepts arbitrary payload bytes after envelope
validation; it does not validate command, input, image or log semantics.
Unknown live transports report UnsupportedTransport; there is no connection,
send, ACK or fake-success route. The cap is a protocol bound, not a phone memory
or latency measurement. Stream reassembly must cap the announced size before
allocating; complete-buffer parsing alone does not implement that receiver.

Root verified24tests, including an independent literal byte array, exact hard
cap acceptance, each rejected encode preserving output, and10k repeated small
parse/encode cycles. Disabling the policy check made its regression RED;
restored source passed. Direct selected test process max RSS2496KiB, rounded
elapsed0.00seconds; Cargo-wrapper peak17280KiB is separate. Neither is exact
heap accounting, leak freedom or real screenshot workload fit. No unsafe or
new native library boundary was introduced by this crate; whole compiler,
host runtime and target integration contracts remain unresolved.

## Existing source interfaces, not deployment proof

### Accepted log-body producer/codec — 2026-10-27 7:25 PM PT

The proposed envelope-v1/Logs body now has a Rust codec and validated producer
constructor. Its fixed38byte header is buildID16 (nonzero), sequence LEu64
(nonzero), monotonic_us LEu64 (zero allowed), severity u8, subsystem-length u8
and message-length LEu32. Followed by exact UTF8 subsystem1..64bytes (no NUL)
and message0..4096bytes (newline/NUL preserved), body39..4198bytes. Application
severity is Trace0, Debug1, Info2, Warning3, Error4, Fatal5, explicitly not syslog
numbering. Core Frame version1+Logs selects this schema; changing the body needs
an actual negotiated version change, not merely editing LOG_BODY_VERSION.

Bare LogRecord::decode is explicitly unbound. from_frame returns FramedLog
retaining envelope boot/correlation plus the body; these are validated fields,
not authentication or replay proof. Module-local errors preserve the original
core Error API. Encoder validates before writes; whole-envelope encoding uses a
bounded4198byte stack scratch body. There is no heap allocation in this codec,
but no stack-budget, persistence, delivery or real-phone fit proof follows.

Root reviewed full source/tests and reproduced48original crate tests, then added
four producer/UTF8-byte-bound/diagnostic-level/repeated-workload tests:52PASS,
0ignored. Actual source mutation replacing the boot binding with a constant
made its metadata test RED, restored GREEN. This remains partial0.2.1; no bank.

Separate source review confirms the voice-bridge Journal<W> writes wall-clock
Unix microseconds and trace/span/payload fields, unlike the new monotonic field.
Do not invent a wall-to-monotonic conversion or silently drop old fields. An
adapter must preserve complete original events/correlations and clock meaning;
there is no such adapter or runtime integration yet. The Linux socket transport
has a pre-parse line bound, whereas Journal::publish's serialization limit is
postallocation. Neither establishes a live Redox logging connection.

At canonical859ad6e, root rechecked tracejournal/src/lib.rs56–166 and README:
publish has a serialized4096byte line limit, per-service rate bounds and64active
service slots; subscriptions use cursors and poll returns retained events plus
a synthetic journal_dropped record after eviction. README describes a256event
memory journal and Linux Unix-socket JSONL; Redox scheme paths are documented
but explicitly not runtime-proven there. Serialization occurs before size
rejection; retained serialized limits are not a complete preallocation or heap
budget. No restart/hard-power-loss persistence follows from an in-memory ring.

Root rechecked rmi4-touch/src/lib.rs159–219: Tracker turns the first contact
into Move/left-button events and release clears a held button; x/y are u16,
not every Contact field. Decode consumes8byte records, filters fingers and
x>1079, and does not establish full screen/rotation/y bounds or input injection.
This is reading contacts, not a CLI gesture or a shared UI broker.

Caller search found the tracejournal Linux binary using Journal::new; other
Journal names in svcd and voice-bridge cannot be assumed to be this type.
Canonical root base/ was absent during caller search. Emitted/vendor Redox
callers and the complete deployment graph remain to be inventoried; a README
path or missing root directory is not proof that a feature is absent everywhere.

## Next bounded contracts, still unimplemented

Keep PLAN0.2.1 OPEN. Define typed request/result bodies and CLI command semantics
for log cursors/gaps, framebuffer dimensions/stride/format/freshness, bounded
command results and timeouts/unknown outcomes, and shared input dispatch with
coordinate transforms and held-input release. Bind build/boot/action/frame
identities; specify pairing/authorization, replay/reconnect and receiver
allocation limits. Then implement fake-transport fault tests, not mock proof
of phone connectivity. Log retention, actual screenshots/commands/taps, real
transport, eventual device integration and observe–act–verify remain separate
gates. No physical resumption is authorized by this host slice.
