# Concrete spill movement and the ownership contract

The audit identifies a conservative allocation guard and inconsistent treatment
of compiler-private memory. It does **not** justify building a general layout
replay framework merely because some selectors query addresses.

A source pointer that may observe compiler scratch could target an ordinary
temporary just as easily as `$AC/$AD`. Staging through that temporary does not
solve the problem. A consistent ownership rule must apply to both locations.
This is the main correction to the earlier explanation of address sensitivity.

## What actually moves

The proposed deletion removes spill IDs 12, 13, 26 and 27 in
`FormatDirectoryRow` (routine 72). At origin `$3000` their planning addresses are
`$332E..$3331`. The remaining spill IDs 52, 53 and 62 move from
`$3332..$3334` to `$332E..$3330`. The allocator's shared sequential cursor also
moves later frame entries. Every moved survivor shifts down by four bytes.

| Case | Removed declarations proposed | Moved surviving entries | Affected routines | Selected removals |
| --- | ---: | ---: | ---: | ---: |
| TN, cart | 4 | 50 | 12 | 0 |
| TN, standalone | 4 | 51 | 13 | 0 |
| Independent pointer-transform control, both runtimes | 2 | 1 | 1 | 0 |
| Argument capture under ZP pressure | 0 eligible | no proposal | no proposal | 0 |

These results repeat at `$41C7`. Counts refer to planning entries, including
views; they are not counts of distinct physical allocations or saved bytes.
TN remains **9,970 bytes cart / 10,077 standalone**. The independent control
remains 99 bytes; argument capture remains 118/200 bytes. The
[receipt](qualification.json) records every changed entry's old/new address,
size, kind and descriptor status in all runtime/origin combinations.

| TN routine | Surviving entries moved |
| --- | ---: |
| `FormatDirectoryRow` | 3 |
| `SetWin` | 17 |
| `PrintNonzeroDigit` | 2 |
| `Xloop` | 2 |
| `Format` | 4 |
| `NewDrive` | 1 |
| `Attrib` | 1 |
| `SwapScr` | 4 |
| `Copy` | 10 |
| `MakeJmp` | 1 |
| `Handle` | 4 |
| `InitPanels` | 1 |
| Runtime `Error`, standalone only | 1 |

The two unused spills in `SetWin` remain excluded from deletion by the existing
exposure rule, but both would move when the earlier four bytes disappear.
`SetWin` also contains arrays, descriptors and initializer references to its
buffers. Those are relocation obligations. Address-taking alone does not imply
that the old numerical address is part of the source-language contract.

## Decisions and remaining coverage

The audit finds the core-with-helpers result matching the selected top-level
materialization. TN has one matching supplier out of three materializations.
Only that supplier contributes selected-core observations; other trials are
reported separately. Exact recorded spill remaps are followed forward. Unknown
epochs, ambiguous suppliers, missing homes and out-of-range references stay
explicit. Matching diagnostic identities does not create an optimizer witness.

The original mixed rounds implicated direct fixed-pointer placement in
`FormatDirectoryRow`, word-array staging and direct word copying in `SetWin`,
and adjacent reload cleanup. Existing candidate attribution was extended to the
first three families to distinguish their local selector checks from other
queries in the same round. Structural-driver and inherited analysis obligations
remain outside those local scopes.

With that attribution, none of these three families' locally applied selectors
has an input mapped to a moved survivor or erased home in TN. Their resolved
inputs refer to unchanged storage. Fixed-pointer placement also makes 16 input
queries against unknown/retired spill homes; these remain unknown, not certified
independent. The apparent moved-input dependencies in the original mixed rounds
belong to other candidates or unscoped analysis. This result repeats in both
runtimes and at both origins.

`Handle` is an instructive counterexample: its selected fixed-pointer placement
compares a global with fixed scratch bytes. That global does not move in this
proposal. A selected rewrite in a moved routine is not necessarily dependent on
the moved storage.

Known-callee word-result placement in `Format` and `SwapScr` already uses the
ordinary-range proof for moved scalar destinations. It does not require their
individual numeric addresses. Other observations include unresolved spill
queries, rejected shapes, shared analysis and outside-driver queries. They are
reported as such, not converted into obligations to preserve old addresses.

The existing late sidecar records selected register copying, pointer retention,
pointer-byte arithmetic, pointer-load sharing, argument X forwarding and
word-load sharing. Its baseline replays pass. The rejected allocation trial
does not reach candidate emission or replay, so the audit makes no claim that
those checks pass or fail for the proposed physical layout.

## Blocker table and implementation decision

| Boundary | Current evidence | Consequence |
| --- | --- | --- |
| Core allocation | Exact surviving-address equality rejects the proposal | An implementation barrier, not proof that those numbers must be preserved |
| Compiler-private storage | Runtime docs call `$AC/$AD` compiler-owned; home definitions treat spills/virtual ZP as non-escaping; late cleanup adds numeric-pointer observability checks for physical ZP | Reconcile ownership before adding more alias proofs |
| Program storage | Later objects and address-bearing initializers move | Preserve object identity, declared aliases and relocations; exposure alone does not pin placement |
| Fixed storage and ABI | Explicit absolute accesses, argument/result slots, runtime and machine-code effects have real address obligations | Retain explicit contracts; do not label all fixed ZP as private |
| Physical emission | Trial stops before candidate emission | Deferred backing, relocations and final cost are not measured for the proposal |
| Diagnostic coverage | Some local gates are attributed; inherited facts and legacy owners are incomplete | Census absence cannot certify independence |

**Decision: resolve the ownership contract first.** Ordinary program accesses
should not observe compiler-private temporaries or active compiler scratch,
regardless of their allocation. Low-level/ABI accesses retain their explicit
contracts. Liveness, overlapping compiler scratch roles, helper clobbers and ABI
reads remain real obligations. The [design addendum](../../MIR6502_LATE_CLEANUP_DESIGN.md#storage-ownership-before-movement-proofs)
distinguishes this proposed rule from existing 6502 behavior and notes the
corresponding explicit rule in the 65816 ABI.

After that decision, identify the genuinely address-dependent operations that
remain. Do not expand per-rewrite replay to preserve accidental scratch addresses.
No semantic ownership change, general compaction or guard relaxation is included
in this diagnostic slice. Pruning stays disabled by default and rollback remains
unchanged. The independent control with only one moving private survivor provides
a smaller implementation target than TN's mix of program storage and scratch.

## Reproduction and validation

The [existing API probe](../../../tools/compare6502/word_result_proof.rs) now accepts
`--spill-movement`. Compile it against the baseline library at `41fc7276` and
the changed library, then capture each into a fresh directory. It runs TN and
both independent controls in both runtimes at `$3000` and `$41C7`, with pruning
enabled in both versions and census-off/on output/work equality assertions.

```sh
build/baseline-probe build/spill-movement-baseline --spill-movement
build/changed-probe build/spill-movement-current --spill-movement
python3 tools/compare6502/spill_movement.py \
  --baseline-api build/spill-movement-baseline \
  --current-api build/spill-movement-current \
  --output build/spill-movement-qualification.json
python3 -m unittest discover -s tools/compare6502 -p test_spill_movement.py
```

The [audit tool](../../../tools/compare6502/spill_movement.py) checks complete
statuses and exact full-output/image/work equality for all 12 cases. Its tests
cover ambiguous equivalent suppliers, rejected-candidate attribution, exact
home remapping and unscoped spill epochs. Raw captures remain under ignored
`build/mir6502-spill-movement/`.

Implementation: `a898ac04`. Validation passed **452 affected MIR6502 unit tests**,
**78 integration tests across five targets**, **three VM tests across two
targets**, and **two Python audit tests**. Eight optional unit checks remain
ignored. Unit coverage includes the full materialization test module, pruning,
and tracing. Integration covers actual LF/CRLF parsing, trace bounds/filters,
candidate attribution and materialization controls. The pruning oracle executes
all byte inputs in both runtimes/origins. VM cleanup/pruning checks ran on the
initial movement diagnostic; the final selector-attribution extension preserves
all 12 complete captured outputs and passes the final integration checks.

The diagnostic also tests later-frame movement, filtered traces with explicit
whole-program delta scope, the 2,048-entry cap and census-off/on work equality.
No optimization, emission, frontend or NIR semantics changed. Validation is
scoped to the diagnostic and its MIR6502 consumers; no full shared-contract
sweep or optional assembler roundtrip was needed.
