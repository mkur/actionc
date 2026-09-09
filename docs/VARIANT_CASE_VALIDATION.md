# Variant CASE: validation on the unmatched path

For a variant with no inline nested variants in any constructor payload, shared
SemIR lowering now combines outer-tag validation with ordinary CASE dispatch.
Each constructor arm already compares the tag against a valid constructor ID.
A final compiler-generated unmatched arm faults with InvalidVariantTag. There is
no separate preliminary validity-range CASE.

For the minimal MaybeByte example, optimized MIR is conceptually:

```text
item.value = 42
item.tag = SOME
tag = item.tag
if tag == NONE:
    PrintE("No value")
else if tag == SOME:
    PrintBE(item.value)
else:
    Error(105)
    halt
```

This is validation fused with dispatch, not removal of invalid-value behavior
or a proof that every typed value is valid. Tag zero and out-of-range tags still
fault. Known-constructor propagation and complete CASE folding are separate
future optimizations, covered by the
[verified NIR propagation plan](NIR_KNOWN_CONSTRUCTOR_TAG_PROPAGATION_PLAN.md).
No new NIR/MIR operation or ABI assumption is introduced.

## Ordering and safety

- Selector preparation and any required aggregate snapshot still happen once,
  before dispatch. Calls, arguments, address evaluation and their effects retain
  their previous order. The internal initialization policy defers only the
  final validation of this CASE selector, not checks within its preparation.
- Scalar, pointer, plain record and union payloads qualify when they contain no
  inline variants. The existing recursive semantic validation query determines
  eligibility, including variants embedded in records/inline arrays.
- Explicit constructor tags dominate payload tests, binder initialization and
  user guards. An invalid tag therefore cannot expose a payload or run a guard.
- Source ELSE and guarded wildcard arms are restricted to the canonical valid
  tag interval, 1 through the constructor count. They remain fallbacks for valid
  unmatched values, not handlers for malformed storage. A false wildcard guard
  proceeds to later source arms in order. The final compiler fault remains
  distinct from user ELSE.
- Any inline nested variant keeps the existing early deep validation, including
  when the payload is ignored with `_`. A valid outer tag alone cannot establish
  nested validity before pattern tests, aggregate binders or guard effects.
  Inactive alternatives are not checked and pointers are not followed.
- LET/assignment/argument/return validation remains unchanged. This does not
  change unconstructed storage, overlap rules, failure-before-publication or
  terminal Error handling.

The optimization happens in shared semantic lowering, so both classic and
MIR6502 benefit. NIR consumes ordinary comparisons, branches, captures and the
existing nonreturning fault. Existing lifetime forwarding may remove the CASE
snapshot when its source is stable; that is an independent proof.

## Measurements and coverage

Compared with `1016b9e`, at origin $3000 for
`tests/support/fresh_maybe_byte.act`:

| Backend | Cartridge XEX bytes | Standalone XEX bytes | Cycles to PrintBE |
| --- | --- | --- | --- |
| Modern classic | 187 → 148 | 457 → 418 | 187 → 163 |
| Optimized MIR6502 | 94 → 80 | 377 → 363 | 46 → 34 |

Printing itself is not executed. The direct construction region remains
10 bytes / 12 cycles. The MIR cost assertions now require no more than 34 cycles
and 80/363 image bytes. Prior aggregate-forwarding CSVs remain historical; this
note does not replace their measurements or attribute their savings to this
change.

`tests/variant_case_validation.rs` checks all four target layouts, single
dispatch/no preliminary range comparisons, final faults, constrained wildcard
and ELSE arms, nested early validation and exactly-once call selectors.
The matching VM test covers tags 0, 1, 2, 3 and 255, skipped guard/ELSE effects,
unexposed payloads, active nested invalidity, inactive payloads and a returning
Error handler across classic/raw/optimized NIR lanes and both Atari runtimes.
Existing maximum-tag, guard mutation, nested-pattern and snapshot tests remain
part of regression coverage.

The raw and optimized `variant_match`, `generic_types` and `case_guards` NIR
snapshots intentionally change: preliminary validation is removed and
wildcards/ELSE are gated before a final fault. This is an executable lowering
improvement, not a printer-only update. Nested-pattern snapshots are unchanged.

```sh
cargo test --test variant_case_validation
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test
cargo check --all-targets
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_fresh_initialization -- --nocapture
```

Verification passed: 3,006 compiler tests (22 existing ignored), all 220 pinned
VM tests (67 library and 153 integration tests across all 47 binaries), NIR
snapshots, all 44 NIR and 167 MIR6502 sweep cases, and all-target checking.
Integration binaries ran in four parallel batches with none omitted. The final
focused compiler/fixture rerun and tightened nine-test fresh-initialization VM
cost suite also pass. `git diff --check` is clean.
