# Initial quantized tensor contract

Status: root-reviewed bounded reader slice of OPEN 0.1.2, not model inference or full
quantized artifact acceptance. No production format conversion or weights exist.

## Payload format and version

The first reader accepts explicitly tagged `GgmlQ8_0LeV1` raw tensor payloads.
This is a project wire-contract version, not a GGUF version or GGML API version.
Layout reference: ggml-org/ggml commit
`da8e2a54b919557b816b1c2beb432ccb766f7ce0`, `src/ggml-common.h`
lines251-256 and `src/ggml-quants.c` lines553-565. These upstream C sources are
read-only layout references, never linked, executed, or runtime dependencies.

Primary references: [block layout](https://github.com/ggml-org/ggml/blob/da8e2a54b919557b816b1c2beb432ccb766f7ce0/src/ggml-common.h#L251-L256),
[dequantization](https://github.com/ggml-org/ggml/blob/da8e2a54b919557b816b1c2beb432ccb766f7ce0/src/ggml-quants.c#L553-L565).
Retained source SHA256: ggml-common.h
`0061131b615c5721fc88a78feeb22c1f8c450f1c2646a317d80796a653bf595c`;
ggml-quants.c `5574a2dccf7c07e75b143733e04a5412d3d8c819e7945f5217a7b83a2b2ff8ab`.

Each block represents32 values: two little-endian IEEE binary16 scale bytes,
then32 signed two's-complement i8 values, exactly34 bytes, without padding.
Decoded value is f32(scale) * f32(quant). Negative and signed-zero finite scales
are valid. NaN/infinite scales are rejected. Subnormal scales must be preserved.
Binary16 conversion uses Rust integer bit operations and f32 multiplication;
"no native math" below means no host C math-library calls, not no arithmetic.
No quantizer is implemented by this slice. Other quantization formats and full
GGUF metadata/container parsing remain unsupported, not implicitly accepted.

## Shape, limits and access

Shape is explicit row-major `[outer..., inner]`, rank1 or greater. Every
dimension is nonzero, inner divisible by32. Reject excessive rank/dimensions,
checked element/byte multiplication overflow, excess configured element/byte
limits, truncation and trailing bytes before scanning scales. Borrow payload
and shape; never allocate a dequantized tensor. Validate every block's scale
at construction. Indexed block decode returns a fixed `[f32;32]` and rejects
out-of-range indices. No unsafe, FFI, native math or new dependency is needed.
No empty tensor support is claimed for this first format.

## Identity and provenance integration gate

The raw reader does NOT authenticate bytes or infer model identity. Before a
future public model-loading path exposes it, require a trusted out-of-band
manifest binding exact LiquidAI/LFM3-1.7B source revision
`40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd`, tensor name/shape/format version,
payload length/SHA256 and original base artifact length/SHA256, plus quantizer
recipe/version and output digest. Reject mismatches before interpreting data.
The base digest must match the previously accepted unquantized manifest entry;
record both manifest identities and their trust source, not a digest copied
from the quantized file. No such production chain is available yet.
Base/source authenticity and secure file opening remain external contracts.
The existing `ArtifactFormat::Quantized` path remains rejected until integration
is reviewed; adding a raw reader alone must not bank0.1.2.

## Acceptance for the bounded reader

Rust-only fixture/test logic. Test exact signed quants and both ends of each
block, normal/subnormal/signed-zero/negative/max-finite scales, all nonfinite
encodings, per-row alignment, multiple blocks/rows, rank/dimension/element/byte
limits, overflow, wrong version, all truncations/trailing bytes, borrowed
pointer identity, out-of-range access and repeated parse/decode. Exhaustively
check finite binary16 conversion against a separate Rust arithmetic reference;
retain independent literal expected values to avoid mirrored-bug acceptance.
Source mutation of scale exponent/sign or quant sign must make tests RED.
Record whole-process RSS separately from heap accounting/leak-freedom/phone fit.

Full artifact integration, real quantized model provenance, operator/cache
parity, pretrained generation and phone fit remain separate unproven gates.

## Review reconciliation — 2026-10-27 4:47 PM PT

Same Perplexity coordinator returned a finite review. Root rejected its claim
that Q8_0 has `dmin` and an offset formula: the pinned header has only `d` and
`qs[32]`, and the pinned dequantizer multiplies them. Negative finite scales
remain valid. Its provenance concern prompted the explicit accepted-base-entry
link above; trust/signature verification stays external. Its arithmetic wording
objection prompted the software-only clarification, not a format change.
No worker verdict clears the full0.1.2 or production provenance gate.

## Next integration packet — 2026-10-27 5:02 PM PT

Implement a private-field `ValidatedBase` proof created only by a constructor
that calls existing validate_artifact on the actual base bytes and trusted
ManifestEntry. It retains the validated artifact and immutable entry plus a
strict64-lowercase-hex out-of-band base-manifest identity. A plain Artifact or
caller-created tuple is not a substitute. This reuses accepted parsing/hash
logic, not a second parser. Manifest identity is a caller trust precondition,
not proof that a signature or canonical manifest serialization was checked.

The typed quantized entry binds exact model/revision, safe simple `.q8`
filename, tensor name, shape, wire tag1, byte length/digest, its own manifest
identity, accepted base manifest identity/name/length/digest, and algorithm
recipe `ggml-q8_0-ref` at the exact layout reference commit above. The recipe
pin identifies the algorithm reference, not an already built Rust quantizer.
No conversion-correctness or production quantizer attestation is claimed.

The validator accepts actual payload bytes, requested name, this trusted entry,
the private validated base and Q80Limits. Bound all manifest strings first
(names255bytes, tensor name1024bytes, exact model/revision/digest/recipe bounds),
and rank/dimensions/element/byte policy before hashing. Require metadata model
and revision equal crate pins; requested name equals entry; base references
equal the validated proof; tensor exists in the base with identical shape and
dtype BF16/F16/F32. Reject missing/mismatched/unsupported data, never fallback.
Verify exact payload hash before Q80 scale interpretation. Return a private-field
validated quantized wrapper with read-only view/identity accessors. Never copy
payload or allocate a dequantized tensor. Keep existing generic
ArtifactFormat::Quantized rejection; the new typed API is the explicit format
path and must not silently accept GGUF/other formats.

Tests use synthetic Rust fixtures, with literal independent expected digests
where asserting payload identity; altered payload/base/name/shape/tag/recipe/
manifest link/model/revision/hash/length and excessive/overflow bounds must
fail. Prove malformed-scale bytes with a wrong digest fail at hash validation;
correctly hashed malformed scale then fails structure. Test failed base bytes
cannot produce the proof, successful borrowed pointers, and repeated workload.
Missing production manifest authentication, secure file opening, quantization
accuracy and weights remain explicit limits even after this API is tested.

Review reconciliation: native review's entry-substitution objection does not
justify cloning: retain the immutable borrowed entry inside the private proof;
existing validate_artifact already requires exact model/revision. Both cannot
vary across accepted entries. Its hex-case objection is false: parse_digest
rejects uppercase, rather than normalizing it. Compare parsed digests as bytes
anyway. Its claim that manifest model checks follow hashing contradicts the
specified ordering; keep them before hashing. Existing base header-byte limits
bound parsed metadata, not the smaller manifest-field caps; retain that limit
explicitly. No native verdict substitutes for implementation/source review.
