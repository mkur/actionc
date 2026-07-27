# MIR6502 TURTLE Optimization Implementation Plan

Status: planned

Date: 2026-07-27

Planning baseline: `cc9b543`

Primary sources:

- `corpora/toolkit/original/extracted/TURTLE.ACT`
- `corpora/toolkit/original/extracted/TURTLE.DM1`

Scope: modern profile, MIR6502 backend

## Objective

Close TURTLE1's remaining size and memory-traffic gap by placing word helper
results directly in their consumers, preserving simple signed-word predicates,
and avoiding homes for caller-private values that can be rematerialized after
known calls.

The changes must be general MIR6502 transformations. They may not inspect
TURTLE routine names, source paths, variable names, or literal addresses.
Classic output is a strategy comparison, not a correctness oracle.

## Baseline Artifacts

Generate the comparison with:

```sh
tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/turtle1-listing-audit-20260727 \
  --no-diffs \
  corpora/toolkit/original/extracted/TURTLE.DM1
```

The audit artifacts are under:

```text
target/turtle1-listing-audit-20260727/turtle/
```

Baseline load hashes:

| Backend | SHA-256 |
| --- | --- |
| modern/classic | `877cbf3dd871fa95214702119847cc9f739203a1c59d2ebeb221a52ccf6fab51` |
| modern/MIR6502 | `d268ec8eca7f1ce659615cfe0b36c08c96d96c34c70ddf4f032a3d27af032918` |

## Baseline Measurements

| Metric | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| XEX bytes | 1,092 | 1,161 | +69 |
| Recognized instruction bytes | 784 | 849 | +65 |
| Data and inline machine bytes | 296 | 300 | +4 |
| Recognized instructions | 338 | 348 | +10 |
| `LDA` | 92 | 107 | +15 |
| `STA` | 82 | 89 | +7 |
| `LDX` | 14 | 24 | +10 |
| `STX` | 5 | 14 | +9 |

MIR6502 allocates four RAM spill bytes and six virtual-zero-page bytes. The
four RAM spills account for 24 accesses. The gap is concentrated in word
result placement and call-crossing preservation rather than emitted program
data.

### Routine concentration

The figures below are recognized instruction bytes.

| Routine | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `Forward` | 151 | 183 | +32 |
| `Turn` | 76 | 90 | +14 |
| `SetTurtle` | 87 | 95 | +8 |
| `TG_ICos` | 141 | 149 | +8 |
| `TG_ISin` | 128 | 132 | +4 |
| `Right` | 27 | 28 | +1 |
| `Left` | 3 | 3 | 0 |
| `TurtleDemo` | 171 | 169 | -2 |

## Finding 1: Word Helper Results Are Routed Through Transient Homes

The pre-home MIR already puts several word operations immediately before their
only direct word store:

- `Turn`: `TG_Phi MOD 360` followed by a store to `TG_Phi`;
- `TG_ISin` and `TG_ICos`: signed multiplication followed by a store to the
  word return slot;
- `Forward`: multiplication followed by stores to `deltaX` and `deltaY`;
- `SetTurtle`: left shifts followed by stores to `TG_CurX` and `TG_CurY`.

The runtime helpers return a word in A/X. Current materialization first writes
the high byte to a virtual or RAM home, writes A to the destination low byte,
reloads the high-byte home into A, and writes the destination high byte:

```text
helper
stx temporary_high
sta destination
lda temporary_high
sta destination+1
```

For a supported structured direct destination this can be:

```text
helper
sta destination
stx destination+1
```

The transformation is local, definition-sensitive, and independent of the
helper operation. It applies to word helpers selected for multiply, divide,
modulo, and variable shifts. It must retain the current conservative exclusions
for absolute/hardware memory and scratch locations whose ordering or aliasing
cannot be proved.

This family accounts for approximately 38 static instruction bytes in TURTLE1.
Some virtual homes may be shared with other values, so the data reduction must
be measured rather than assumed.

## Finding 2: A Direct Global Signed Word `< 0` Loses Its Sign-Byte Form

`Turn` contains:

```action
WHILE TG_Phi<0
```

MIR retains a signed word comparison with zero, but the signed-zero
compare-to-branch selector accepts parameters, locals, statics, spills, and
zero-page cells while excluding an ordinary structured global. TURTLE1
therefore subtracts zero from both bytes and performs signed-overflow
normalization before branching.

For an adjacent load from a stable structured global, signed word `< 0` and
`>= 0` depend only on bit 7 of the high byte. The existing selector can emit:

```text
lda global+1
bmi target
```

The global extension must use the same producer/consumer, reaching-definition,
alias/effect, and machine-state checks as the existing local/static form.
Absolute addresses and hardware-visible memory remain excluded. The expected
TURTLE1 saving is 12 instruction bytes.

## Finding 3: Caller-Private Word Loads Are Spilled Across Known Calls

`Forward` uses its `length` parameter in two expressions:

```action
deltaX=length*TG_ICos(TG_Phi)
deltaY=length*TG_ISin(TG_Phi)
```

The pre-home MIR loads `length` before each known call and consumes it only
after the call. Materialization consequently copies both bytes to RAM spills
before the call and reloads them for the multiplication.

A known callee with structured effects that cannot write caller-private
parameter or local storage cannot invalidate such a load. A single-use direct
load should therefore be sunk or rematerialized after the call instead of
receiving a call-crossing home. Calls with unknown effects, machine blocks,
absolute memory, pointer writes, and aliasable storage remain barriers.

The two occurrences in `Forward` account for most of its 16 RAM-spill accesses
and are expected to save roughly 20-24 instruction bytes while removing
`spill0` and `spill1`.

## Finding 4: Two Helper Results Form the Final `Drawto` Arguments

After the preceding slices, `Forward` may still materialize the two
`RSH 7` results through homes before the final known machine call. The first
result must survive the second shift helper, while the second can move directly
to Y.

This is a candidate for helper-result-to-call-argument placement using exact
helper clobber summaries and the known callee ABI:

- retain the first word result in the destination A/X argument cells only when
  those cells are not clobbered by the second helper;
- move the second result low byte directly to Y;
- avoid public argument shadow stores that the known machine call does not
  demand.

This slice is conditional. Implement it only if a post-Slice-3 listing still
shows a material family and the required preservation facts are already
available from shared call/helper summaries.

## Decisions Fixed by This Plan

1. Helper result placement belongs in MIR6502, not NIR.
2. The helper ABI's word result is A=low, X=high; direct word consumers may use
   both registers when the destination and effects are safe.
3. Structured globals are eligible for sign-byte comparisons. Arbitrary
   absolute and hardware-visible memory are not.
4. Caller-private parameters and locals may be rematerialized across only a
   known call whose structured effects prove that storage cannot be written or
   aliased.
5. Unknown calls, pointer writes, machine blocks, helper scratch conflicts, and
   absolute memory remain barriers.
6. No NIR change is planned. If the necessary width, storage identity, or call
   effect is absent from verifier-clean MIR, stop and amend this note rather
   than consulting SemIR from MIR6502.
7. Every behavior-changing slice receives focused emitted-shape tests, an
   execution-level runtime check, a fresh TURTLE1 audit, and its own commit.

## Implementation Slices

### Slice 0: Baseline and execution coverage

Status: complete.

- Add a mandatory real-source TURTLE1 quality test with a 1,161-byte baseline
  cap and assertions for the measured helper-result and spill families.
- Add a compact VM fixture covering word multiply, modulo, shifts, signed
  direct-global zero relations, and a caller-private word load consumed after a
  known call.
- Run the fixture under modern/classic and modern/MIR6502.

This slice must be code-size neutral.

Result:

- Added a mandatory real-source TURTLE1 materialization and 1,161-byte output
  gate.
- Added a modern/classic and modern/MIR6502 VM oracle for multiply, modulo,
  variable left shift, signed global zero relations, and a caller-private word
  parameter consumed after a known call.
- The TURTLE1 output remains byte-identical to the planning baseline.

### Slice 1: Direct word-helper result stores

- Extend the shared word-store consumer selector to recognize a word binary
  operation that selects a runtime helper and is immediately stored to a
  supported direct word destination.
- Emit the helper followed by low-byte `STA` and high-byte `STX`.
- Feed selected helper requirements through the existing transactional rewrite
  candidate rather than mutating helper state before proof acceptance.
- Add positive tests for multiply, divide/modulo, and shifts, plus rejection
  tests for reused definitions and unsafe destinations.
- Re-run the VM fixture, full tests, and TURTLE1 audit.

### Slice 2: Structured-global signed-zero branches

- Admit a stable structured global in the signed-word-zero load/source checks.
- Reuse the existing sign-byte compare-to-branch selection and shared proof
  framework.
- Test `< 0`, `>= 0`, reversed operands, and rejection across effects or
  absolute memory.
- Re-run the VM fixture, full tests, and TURTLE1 audit.

### Slice 3: Rematerialize caller-private loads after known calls

- Identify a direct parameter/local word load with no pre-call use and a
  post-call single-use consumer.
- Use known-callee structured effects and storage identity to prove the load
  stable across the call.
- Move or clone the load after the call before home demand is finalized.
- Keep multi-use values and every unknown/aliasing case unchanged.
- Add CFG, positive, and conservative rejection tests.
- Re-run the VM fixture, full tests, and TURTLE1 audit.

### Slice 4: Residual helper-result argument placement

- Re-audit `Forward` after Slice 3.
- If still justified, select the two shifted word results directly into the
  final known call's A/X/Y placements using exact helper and callee summaries.
- Do not add a TURTLE-specific or source-call-order rule.
- Re-run all coverage and record the measured result.

### Slice 5: Final audit

- Generate a fresh classic/MIR6502 listing, compact MIR, materialized MIR,
  quality report, spill report, and per-routine comparison.
- Record accepted/rejected counts for each new selector.
- Update this note with final sizes, hashes, routine deltas, and remaining
  opportunities.
- Run the complete Toolkit modern/MIR6502 sweep and report any changed files.

## Required Validation

After each behavior-changing slice:

```sh
cargo fmt --check
cargo test --lib mir6502
cargo test --test mir6502_turtle_quality
fixtures/runtime/run-turtle-word-placement-vm.sh
tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/turtle1-listing-audit-20260727-sliceN \
  --no-diffs \
  corpora/toolkit/original/extracted/TURTLE.DM1
```

Before marking the plan complete:

```sh
cargo test
surveys/toolkit/compile-toolkit-batch.sh --profile modern --backend mir6502
```

If NIR or semantic lowering changes despite the current plan, also run the NIR
fixture and sweep checks required by `AGENTS.md`.
