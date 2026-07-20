# MIR6502 rewrite workflow baseline

Date: 2026-07-20

Compiler revision: `eeb20df` (`docs: plan routine-aware MIR6502 rewrites`)

Scope: `samples/tn/modern/TN.ACT`, `--profile modern --backend mir6502`

This freezes the output and migration inventory before the shared analysis and
rewrite workflow is introduced. The artifacts are generated from a clean,
detached worktree at the revision above; unrelated public-ABI work in the main
worktree is deliberately excluded.

## Reproduction

Run each command from a clean checkout of `eeb20df`:

```sh
mkdir -p target/tn-rewrite-workflow-baseline

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/tn/modern/TN.ACT \
    > target/tn-rewrite-workflow-baseline/TN.lst \
    2> target/tn-rewrite-workflow-baseline/TN.peepholes

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-rewrite-workflow-baseline/TN-materialized.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-rewrite-workflow-baseline/TN-pre.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/tn/modern/TN.ACT \
  > target/tn-rewrite-workflow-baseline/TN.xex

cargo run --quiet --bin actionc-listing-quality -- \
  target/tn-rewrite-workflow-baseline/TN.lst \
  > target/tn-rewrite-workflow-baseline/TN.quality
```

The source hash is
`097df477534d50b9aaec1d733b8d6a66f6792e00cd7703e46331c2d5425f8797`.
The `Cargo.lock` hash is
`02e7e9e564916b19fe9aad0dc7d2efd44fe9fc58cf56132022ebddbbc4a754fd`.

## Artifact manifest

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `TN-pre.mir` | 135,966 | `31af8f9332d457b8d34f4e56c5f3c7bda21d41bbe0b5e9af6b2ba9fb06b7f1f4` |
| `TN-materialized.mir` | 172,780 | `7f6740f4b31b13ae189bb7447af8f0b54b50fc7b853c612d338f57be8f92c17a` |
| `TN.lst` | 165,622 | `5249f4aac72023b19ebc368ba5acd4a244d8c51aed36d27122bd7014a9bb60fd` |
| `TN.xex` | 12,719 | `844defe91c2714fe2133eb7f3fb1b7d351e6290ba70607c716dc73755a2fa785` |
| `TN.peepholes` | 268,107 | `f8085465facbb9f05c73ff3950ac73db0ec6f54d346b59ffd7c7279cda3c4df0` |
| `TN.quality` | 3,361 | `e25092b9fb250d5caa6b0ea75a8994ef9cd8ec5c2b9b56b9282d9010a6e50dd8` |

The load file contains a 12,707-byte primary segment plus 12 XEX framing and
run-vector bytes.

## Listing and transformation summary

The quality report records 5,233 recognized instructions and 11,715
recognized instruction bytes. It includes 1,533 `LDA`, 1,191 `STA`, 200 `JMP`,
369 `JSR`, 37 branch-over-`JMP` shapes, 27 spill data labels, and 28 adjacent
store/reload pairs. These parser-derived counts are comparative; XEX segment
sizes are authoritative.

Selected optimizer counters establish the migration baseline:

| Counter | Count |
| --- | ---: |
| compare operand consumer before branch | 112 |
| delayed byte-index producer suppression | 53 |
| byte-store consumer | 75 |
| direct-copy store consumer | 41 |
| call-result store consumer | 34 |
| word-store consumer | 28 |
| call-argument expression consumer | 21 |
| address-store consumer | 11 |
| word-load address forwarding | 71 |
| pre-home fixed-point removals | 172 |
| MIR copy-propagation uses | 234 |
| MIR copy-propagation dead temp definitions | 97 |
| staged byte/word update | 3 |
| staged word-store forwarding | 1 |
| word-array value staging | 3 |
| indirect compound/constant/direct-store | 1 / 2 / 1 |
| SSA-lite cross-block forwards | 3 |
| SSA-lite redundant reloads | 65 |
| dead scratch stores / dead register writes | 12 / 5 |
| gross / retained cross-block home-demand lanes | 8 / 1 |

The full report hash above covers all aggregate and per-site counters.

## Runtime smoke record

The baseline is packaging-clean: its XEX is accepted by the repository's ATR
packaging path and the Atari800 launch-argument tests pass. There is no
automated screen oracle for TN, so interactive startup is recorded separately
from the deterministic manifest: the current TN/liveness-fix lineage was
manually booted through DOS and reached the TN main screen on 2026-07-20. Every
later output-changing slice must repeat that manual smoke or explicitly mark it
pending; byte-identical infrastructure slices inherit this result.

## Migration inventory

The machine-readable checklist is
[`mir6502-rewrite-migration-inventory.tsv`](mir6502-rewrite-migration-inventory.tsv).
It assigns every producer-removing entry point identified by the liveness audit
to its rewrite phase and migration batch, and also records already guarded
reference implementations. `tests/mir6502_rewrite_inventory.rs` checks both
that every audited entry point is listed and that every listed source function
still exists. Remove that temporary test only after all rows are migrated to
the shared rewrite workflow and the inventory is retired.
