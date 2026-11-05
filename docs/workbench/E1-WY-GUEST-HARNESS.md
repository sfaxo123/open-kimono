# Wyoming guest harness: current contract

Updated2026-10-25 10:55AM PT. Canonical ownership/outcome: docs/HANDOFF.md.
Historical rejected authoring receipt is retained at c530189 in Git.

Production uses the accepted staged image directly READONLY, no restaging,
copies or mounts. Phase admission requires --approve and matching WG_CODEX_PHASE;
this is bookkeeping, not an authentication boundary. No actual VM proof is
implied by source or stub tests. Each VM phase needs explicit Codex review/budget.

The actual positional guest CLI, proven native QEMU argv, prompt-driven serial,
real pinned Python reference echo and silent listeners, nonce-bearing complete
TCP-flow capture, timed exact read-timeout classification, owned-group cleanup
and protected pre/post hashes form the acceptance contract. No phone proof.

Host acceptance: original repair a09b737 independently48/48PASS. Codex corrected
raw ANSI/OSC serial offsets, enforced exact clean reference HEAD before claim,
and fixed valid contained retransmits. Final host suite50/50PASS at
$HOME/artifacts/handoff/redox-perplexity/wy-root-final-host-tests-20261025T1752.log.
Focused offset/retransmit suite verifies6 cases; Luna reviewed final delta with
no concrete blocker to ONE90s readonly guest attempt. Results still pending.

Files: e1-wy-guest-harness.sh, -fixture.py, -pcap.py, -tests.sh,
-offset-tests.py under scripts/qemu-demo. Run host tests with bash on -tests.sh
and python3 on -offset-tests.py. Use results.tsv/rawserial/pcap/fixture receipts,
not a test count or worker claim, for actual guest acceptance.
