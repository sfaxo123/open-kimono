# Pinned tokenizer source contract

Source-only readiness, 2026-10-27 5:33 PM PT. No tokenizer implementation,
template renderer, encoding parity or model generation is accepted here.

Model remains LiquidAI/LFM3-1.7B at
`40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd`.
The missing `tokenizer.json` was retrieved from the exact revision:
`https://huggingface.co/LiquidAI/LFM3-1.7B/resolve/40ebe0aa6662c9d40389c2f5d565b6a2c6aebacd/tokenizer.json`.
Pinned Hub tree metadata reports17,908,635bytes and LFS SHA256
`7e226e34bface23ccf6e495d157bf77186c1c30a7659880e1a8994db528c3701`;
the downloaded bytes independently matched that digest and length before being
copied to the existing `artifacts/handoff/redox-perplexity/lfm3-source` cache.
Official HTTPS/commit identity is the source trust boundary, not a signature
verification claim. No model weights or dependency packages were downloaded.
Tree/download/size/hash evidence is RUST-CONTINUE-20261028T0000-* in artifacts.

## Actual data facts, not guesses from the backend name

- JSON version1.0, BPE model with124,893 vocabulary entries, IDs0..124892.
- 293,320 ordered merges; `ignore_merges=true`, `byte_fallback=false`.
- 140 added tokens with IDs124893..125032. Added-token strip/normalization/special
  flags, exact strings and collision policy require faithful implementation.
- Normalizer null. Pre-tokenization is Split/Isolated then ByteLevel with
  `add_prefix_space=false`, `trim_offsets=true`, `use_regex=false`.
- The actual Split regex includes Unicode categories, contraction alternatives,
  case-insensitive grouping and negative lookahead `\s+(?!\S)`. Do not assume
  the basic Rust regex crate supports that syntax or silently simplify it.
- ByteLevel post-processor and decoder have their own distinct flags; preserve
  byte/unicode mapping, special-token behavior and streaming decoding.
- Cached tokenizer_config.json names TokenizersBackend and special strings;
  it does not itself establish the BPE model/vocabulary above. Its
  model_max_length is1000000000000000019884624838656, which exceeds u64.
  Preserve the exact metadata (e.g. u128) and apply explicit bounded service
  context policy; never cast this sentinel into a buffer size or allocation cap.
- Cached chat_template.jinja contains real generation/tool/reasoning macros.
  Marker counting or a hash-only check is not a renderer or output parity.

## Next bounded work

Source clarification, 2026-10-27 5:42 PM PT: added-token flags are not uniform.
`<think>`124901, `</think>`124902 and `<|tool_call_start|>`124905 are
`special=false`; `<|im_end|>`124900 is `special=true`. Preserve the explicit
per-token flags rather than infer them from spelling. Actual model keys include
`unk_token`, not `unk_id` or `special_tokens`; an absent key is not evidence of
an explicit null value. Root rechecked these facts with direct source viewing.
The native coordinator's new inline Python inspection recipe violated the
Rust-only brief and is not an accepted reusable recipe or test implementation.

First Rust artifact/index slice should validate exact source identity, bounded
JSON/strings/counts, duplicate IDs/names, merge references/ranks and added-token
flags, then expose actual ID/content/rank lookups against the pinned asset.
Synthetic malformed-input tests supplement a real-asset acceptance case; they
do not replace it. No encoding/decoding completion claim follows from indexing.
Separately freeze the exact pre-tokenization/ByteLevel/template behavior before
their implementations. New implementation/test/generator logic remains Rust,
without a Python/C/FFI tokenizer or remote-engine fallback. Existing pins and
native/unsafe dependency boundaries remain review gates. Full0.1.3 stays OPEN.
