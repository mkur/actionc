# Variant CASE: validation on the unmatched path

For a variant with no inline nested variants in any constructor payload, shared
SemIR lowering now combines outer-tag validation with ordinary CASE dispatch.
Each constructor arm already compares the tag against a valid constructor ID.
A final compiler-generated unmatched arm faults with InvalidVariantTag. There is
no separate preliminary validity-range CASE.

When the captured tag is unknown, the normalized dispatch is conceptually:

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

Unknown, zero and out-of-range tags retain the final fault behavior. Verified
NIR can now replace a tag load when executable stores prove its exact U8 byte
in private aggregate capture storage. For `LET item=MaybeByte.SOME(42)`, the
optimized computation keeps the construction stores and calls `PrintBE(42)`;
the unreachable NONE and invalid-tag arms disappear.

This proof uses stable storage IDs and byte offsets, independently of constructor
names. Equal incoming facts survive CFG joins, and exact nonvolatile copies
transfer facts to independent snapshots. Calls, opaque effects, volatile access
and unresolved memory clear memory facts. A selector already captured in SSA
keeps its value across a later guard call. Each nested tag needs its own proof;
a known outer tag supplies no assumption about the payload's validity.

The [implementation plan and measurements](NIR_KNOWN_CONSTRUCTOR_TAG_PROPAGATION_PLAN.md)
record the bounded scope and regression matrix. No new executable NIR/MIR form
or ABI assumption is introduced. MIR6502 reaches PrintBE in 23 cycles with
45-byte cartridge / 281-byte standalone images, versus 34 cycles and 80/363
bytes before propagation; printing is excluded.

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

Validation/dispatch fusion happens in shared semantic lowering, so both
classic and MIR6502 benefit. Byte-constant propagation runs only in verified NIR. NIR consumes ordinary comparisons, branches, captures and the
existing nonreturning fault. Existing lifetime forwarding may remove the CASE
snapshot when its source is stable; that is an independent proof.

## Measurements and coverage

Historical validation/dispatch fusion, compared with `1016b9e`, at origin $3000 for
`tests/support/fresh_maybe_byte.act`:

| Backend | Cartridge XEX bytes | Standalone XEX bytes | Cycles to PrintBE |
| --- | --- | --- | --- |
| Modern classic | 187 → 148 | 457 → 418 | 187 → 163 |
| Optimized MIR6502 | 94 → 80 | 377 → 363 | 46 → 34 |

Printing itself is not executed. The direct construction region remains
10 bytes / 12 cycles. Subsequent verified-NIR byte propagation lowers the MIR
row to 45/281 image bytes and 23 cycles, with 21 bytes of Main code; current cost
assertions pin those limits. Classic retains the table's final measurements.
Prior aggregate-forwarding CSVs remain historical.

`tests/variant_case_validation.rs` checks all four target layouts, single
dispatch/no preliminary range comparisons, final faults, constrained wildcard
and ELSE arms, nested early validation and exactly-once call selectors.
The matching VM test covers tags 0, 1, 2, 3 and 255, skipped guard/ELSE effects,
unexposed payloads, active nested invalidity, inactive payloads and a returning
Error handler across classic/raw/optimized NIR lanes and both Atari runtimes.
Existing maximum-tag, guard mutation, nested-pattern and snapshot tests remain
part of regression coverage.

The validation-fusion change updated raw and optimized `variant_match`,
`generic_types` and `case_guards` snapshots: preliminary validation is removed and
wildcards/ELSE are gated before a final fault. This is an executable lowering
improvement, not a printer-only update. Nested-pattern snapshots are unchanged.

`tests/nir_subregion_constants.rs` covers exact byte identity, joins, loops,
copies, overlap, escaped addresses, data relocations, aliasing, effect barriers,
known-invalid bytes and unknown nested tags. The dedicated
`tools/vm-runtime-tests/tests/known_constructor_tags.rs` covers fresh/nested
constructors, mutating guards, repeated initialization, volatile bus order and
terminal faults. The new `known_constructor_tags.optimized.nir` fixture
intentionally folds tag dispatch; existing lowering snapshots remain stable.
Four-target checks establish NIR/layout correctness. Atari VM execution does
not imply native runtime Error-adapter support.

```sh
cargo test --test variant_case_validation --test nir_subregion_constants
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test
cargo check --all-targets
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --locked --test aggregate_fresh_initialization -- --nocapture
```
