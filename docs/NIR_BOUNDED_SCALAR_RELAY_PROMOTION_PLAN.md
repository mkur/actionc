# NIR Bounded Scalar-Relay Promotion Plan

Status: implemented 2026-09-05. Created 2026-09-04.

This plan adds the general optimization needed to compose a pointer byte load
with a following static-table lookup. The motivating AES `SubBytes` source is:

```text
value = state(i)
value = sbox(value)
state(i) = value
```

The current optimized NIR retains both stores to and loads from `value`.
Materialized MIR therefore emits two absolute stores and one absolute indexed
reload around the table lookup. The desired 6502 shape is:

```text
LDY i
LDA (state),Y
TAY
LDA sbox,Y
LDY i
STA (state),Y
```

The implementation must not recognize AES, `sbox`, or `SubBytes` by name. It
must expose a general, short-lived scalar value chain and let the established
MIR6502 index and register-home selectors choose the target instructions.

## Architectural decision

The staging object in AES is a source local, not a compiler spill. Extending
post-home spill forwarding would therefore miss the motivating case, while
treating arbitrary locals as private spills would weaken alias and visibility
rules.

Instead, extend the existing NIR storage-promotion policy with a bounded-relay
tier. NIR storage analysis already proves that a local is ordinary scalar
storage, not address-taken, not machine-visible, not alias-backed, safe across
calls, and not needed at routine exit. The existing `promote_home` SSA renamer
already removes the local accesses while preserving CFG and invocation
semantics. The new work is a conservative profitability decision, not a second
promotion implementation.

MIR6502 already has the required composition path. Indexed-address selection
turns a byte index into a Y consumer, and home-demand selection can retain a
single-use accumulator producer in A. A move from A to Y emits `TAY`. No new
MIR operation or emitter peephole is planned.

## Correctness and profitability contract

A bounded scalar relay is eligible only when:

- shared storage analysis says the home is promotable;
- it is an ordinary one-byte local scalar;
- every direct access is in one reachable basic block;
- the first access is a store and accesses alternate store/load;
- there are at least two store/load pairs;
- no store-to-load interval contains more than three intervening operations;
- each stored definition is therefore read exactly once before replacement;
- the access sequence does not cross a call, foreign-code operation,
  unsupported operation, or volatile barrier; and
- promotion needs no entry seed, block parameter, or exit synchronization.

These conditions keep the tier distinct from hot-home and indexed-induction
promotion. In particular, the existing cold one-store/one-load pressure guard
remains unchanged.

Promotion may lengthen the lifetime of the stored SSA value only within the
bounded block-local interval which formerly held that value in the local. It
does not remove, duplicate, or reorder pointer, table, volatile, or hardware
accesses.

## Slice 1: select bounded relay homes

Add a small classifier to `nir::promotion` and include its result in the
existing home selection predicate. Reuse `promote_home` without changing its
renaming or synchronization rules.

Focused tests cover:

- a two-definition byte relay selected despite being below the hot threshold;
- the existing single-definition cold home remaining unpromoted;
- multiple reads of one stored definition;
- a cross-block lifetime;
- calls, foreign code, unsupported operations, and volatile barriers;
- an address-taken or otherwise non-promotable local; and
- word storage remaining outside the first implementation.

Suggested commit: `nir: promote bounded scalar relay homes`.

## Slice 2: verify MIR accumulator-to-index composition

Add a source-level MIR6502 fixture for a pointer load, static byte-table lookup,
and pointer store through a relay local. Verify at the relevant stages that:

- optimized NIR contains no direct access to the relay local;
- materialized MIR carries the pointer-load result from A to Y;
- the static table uses absolute indexed-Y addressing;
- the original pointer index is restored before the final indirect store; and
- no relay spill or local access remains.

If the fixture exposes a failure, fix only the existing generic handoff between
index materialization and register-home selection. Do not add a sequence- or
benchmark-specific matcher.

Suggested commit: `mir6502: verify accumulator-to-index composition`.

## Validation

Run:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo run --bin actionc-mir6502-sweep -- fixtures/mir6502
cargo test
```

Then rebuild the standalone Atari AES benchmark, verify its checksum, inspect
the `SubBytes` listing, and record XEX size and PAL timing. The motivating
sequence should save about eight static bytes and ten cycles per substituted
byte, approximately 1.15 million cycles over the current workload. These are
measurement targets, not test assertions.

## Implementation outcome

The bounded-relay classifier is implemented in `nir::promotion` and reuses the
existing storage analysis and `promote_home` SSA renamer. No MIR6502 rewrite or
emitter special case was needed: the established accumulator and Y-index
materialization selected the intended instruction sequence once the relay home
was removed from NIR.

The source-level MIR6502 regression fixture checks the pointer-load-to-table-
index handoff, the final pointer store, and the absence of relay locals or
spills. Positive and negative NIR unit tests cover the bounded two-definition
case, multiple reads, cross-block values, address escape, word storage, and
call, foreign-code, unsupported-operation, and volatile barriers. The fixture-
corpus total was updated for the additional source.

On the standalone Atari AES benchmark, the change produces:

- XEX size: 4048 -> 4039 bytes (9 bytes smaller);
- elapsed time: 1777 -> 1729 PAL ticks (48 ticks, or 0.96 seconds, faster);
- checksum: `SUM: 0` before and after; and
- `SubBytes`: two absolute stores and one absolute reload become one `TAY`.

The complete `cargo test` suite and both NIR/MIR6502 fixture sweeps pass.
