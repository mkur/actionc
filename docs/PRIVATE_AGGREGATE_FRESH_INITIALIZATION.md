# Fresh private aggregate initialization — slice 2

Implemented and measured 2026-09-09, relative to `191aa2a` plus the pending
variant-storage-contract and slice 1 analysis working-tree changes. Historical
[slice 1 measurements](PRIVATE_AGGREGATE_FORWARDING_BASELINE.md) are unchanged.
This is shared semantic lowering, not a new target peephole or general NIR
copy-forwarding pass.

## Boundary and supported producers

`lower_binding_statements` now uses a distinct fresh-initialization entry point
in `src/semantic/ir/fresh.rs`. `initialize_aggregate_value` shares preparation
between final fresh homes and compiler-created captures. Ordinary stores,
complete `RecordCopy`/`CopyBytes`, calls and validation implement the result;
classic and all MIRs consume the same semantic decision.

- Eligible variant constructors write active fields left-to-right, zero only
  uncovered ranges and write the tag last, directly into the LET home.
- Eligible nested constructors initialize their canonical inline destination.
  Complete ordinary record/union payloads take one necessary full-image copy,
  including array bytes, padding and bytes beyond a smaller union member.
- `LET saved=original` remains a snapshot, never an alias. Direct ordinary
  sources need one complete transfer, without destination/source pointer
  relays. Checked sources are validated before the copy, so failure does not
  publish the binding or let an Error handler observe a partial new value.
- A whole native automatic binding can be `SemCall.aggregate_result`, with
  ordered callee/argument preparation and result validation intact. Recursive
  calls have distinct activations and cannot address the unpublished binding.
  The callee's return capture/physical hidden-result transfer is unchanged.

The constructor proof is bounded and positive: literals, canonical integer/enum
constants, integer expressions other than division/remainder, and ordinary
direct sources/inline fields. It checks the complete producer, so an earlier
inert nested field is not redirected across a later effectful argument.
Backing facts come from declarations already lowered, ordinary scalar/value
parameters and earlier LET bindings, keyed by SymbolId. A declaration not yet
seen is unknown and conservatively retains the previous lowering.

Calls, possible arithmetic/validation faults, REAL operations, volatile or
alias/absolute reads and pointer/index reads do not pass the constructor proof.
Ordinary record/union copies already needing just one transfer retain that
path; uncertain checked sources retain address capture and overlap checking.
Nested call results remain separate **whole** buffers, even on native targets:
the aggregate ABI verifier deliberately still rejects subobject result places.

Atari's routine-static call-result bindings remain staged. Default user-call
`SemEffects` are not proof of purity or nonreentry. Extending this requires a
structured call/lifetime proof, not a source-name exception. Effectful native
constructors also conservatively retain staging in this bounded slice, even
where a future proof could admit more cases.

Source replacement still preserves old-value visibility during RHS effects and
on failure. Static declaration images, LET scope/lifetime rules, CASE captures,
guards and validation are unchanged. Fresh initialization remains at its source
execution point: skipped branches do not write, loops reinitialize and shadowed
names have separate storage identities.

## Minimal example

Reproducible source: `tests/support/fresh_maybe_byte.act`.

```action
TYPE MaybeByte=VARIANT [NONE SOME [BYTE value]]
PROC Main()
  LET item=MaybeByte.SOME(42)
  CASE item OF
  WHEN MaybeByte.NONE THEN
    PrintE("No value")
  WHEN MaybeByte.SOME(value) THEN
    PrintBE(value)
  ESAC
RETURN
```

The LET region, at origin $3000 with absolute-addressed storage, is now:

```asm
LDA #42
STA item+1
LDA #2
STA item
```

Four instructions, **10 bytes / 12 CPU cycles**, on classic and raw/optimized
NIR/MIR6502 paths in both runtimes. The prior MIR region was 48 bytes / 70 cycles.
The constructor home and destination-pointer home (two bytes each) are absent;
the initialization has no `CopyBytes`. The subsequent CASE still takes its own
snapshot and validates it. MIR also reuses the tag accumulator value in that
remaining copy, through existing optimization.

| Full example metric | Classic before → after | MIR6502 before → after |
| --- | --- | --- |
| Main instruction bytes, excluding local data | 217 → 155 | 102 → 61 |
| CPU cycles from entry to PrintBE entry | 316 → 187 | 118 → 56 |
| Cartridge XEX bytes | 253 → 187 | 148 → 103 |
| Standalone XEX bytes | 523 → 457 | 431 → 386 |

Printing itself is not executed or timed; A=42 is checked at PrintBE entry.
These are CPU cycles, not PAL frames. Whole-example measurements use the public
modern classic/MIR compile modes; the isolated four-instruction test separately
covers classic and all three raw/optimized NIR/MIR configurations.

## Shared corpus after this slice

Full data: [four-target NIR](PRIVATE_AGGREGATE_FRESH_NIR.csv) and
[Atari VM/initial MIR](PRIVATE_AGGREGATE_FRESH_VM.csv). The source/oracle and
measurement definitions are the same as the frozen slice 1 baseline (96 NIR
rows, 72 VM rows, 288 guarded VM executions).

| Cartridge optimized-MIR case | XEX bytes before → after | Cycles before → after |
| --- | --- | --- |
| Variant capture chain | 616 → 452 | 793 → 523 |
| Variant snapshot then mutation | 657 → 575 | 926 → 791 |
| Variant fresh call | 378 → 378 | 441 → 441 |
| Record/union fresh call | 272 → 272 | 352 → 352 |

The Atari snapshot improvements remove checked-source pointer relays, not real
snapshot copies or validation. Record/union corpus costs are unchanged there.
Native fresh-call cases lose one logical full-value copy and one capture home:
e.g. 68k record capture bytes 9 → 6, union 12 → 8, variant 16 → 12. These sums
are logical allocated extents, not final frame sizes. Physical ABI transfers
remain; no native execution or missing-Error-adapter support is claimed.

## Regression surfaced and corrected

The large record/union payload test exposed scalar promotion appending a reload
after a nonreturning `NirCallee::Fault`, and potentially an exit synchronization
store. This violates the existing terminal-fault verifier contract. Promotion
now synchronizes observable routine-static values **before** the fault and adds
neither a reload nor a normal-exit store afterwards. Invocation-private values
need no fault synchronization. A focused promotion test covers both activation
models; the large-payload VM test exercises the original failure. Fault effects,
handler dispatch and verifier requirements have not been weakened.

Raw and optimized `nested_patterns` NIR snapshots intentionally change: the
inner constructor writes into its private enclosing capture instead of taking
another capture/copy. The enclosing replacement and subsequent CASE copies
remain. This is a lowering improvement, not a printer-only update.

## Reproduction and verification

```sh
cargo test --test aggregate_fresh_initialization
cargo test --lib promoted_homes_sync_before_a_fault
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_fresh_initialization -- --nocapture
cargo test --test aggregate_forwarding_baseline -- --nocapture
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_forwarding_audit -- --nocapture
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo check --all-targets
cargo test
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked
```

Focused compiler coverage includes all four layouts, direct initialization,
complete copies, native/static calls, whole-result ABI boundaries, conservative
effects/backing exclusions and shadowing. The new VM suite checks byte-poisoned
homes and tag-last writes, skipped bindings, reentry before publication,
loop/shadowing execution, full record/union images, mutation, argument ordering,
and invalid snapshots/nested payloads with returning Error handlers.

Verification passed: 2,983 compiler tests (22 existing ignored tests), all 44
NIR and 167 MIR6502 sweep cases, NIR snapshots, `cargo check --all-targets`,
the full pinned 207-test VM run, and the expanded nine-test fresh-initialization
VM suite (including the additional PrintBE cost probe). The shared corpus's
288 VM executions also passed. Final promotion-only and fresh-lowering compiler
reruns passed; `git diff --check` is clean.

Broader NIR coalescing, CASE snapshot reuse and physical return-slot forwarding
remain slices 3–5 of the implementation plan.
