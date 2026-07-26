# MIR6502 KALSCOPE Optimization Implementation Plan

Status: complete; Slices 0-6 implemented and audited

Date: 2026-07-26

Planning baseline: `812a70e`

Primary source:
`samples/toolkit/modern/KALSCOPE.DEM`

Scope: modern profile, MIR6502 backend

## Objective

Close the current KALSCOPE gap by improving general MIR6502 handling of:

- a private word pointer that is updated, committed to its source home, and
  immediately reused by an indirect access;
- word arithmetic whose result feeds a bitwise word operation;
- two pure byte expressions passed in A/X;
- direct byte values delayed behind indirect-address preparation;
- known runtime-helper scratch effects where they enable safe destination
  preparation.

No rewrite may inspect KALSCOPE routine names, source paths, variable names, or
literal table addresses.

## Baseline Artifacts

Generate the common artifacts with:

```sh
tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/kalscope-listing-audit-20260726 \
  --no-diffs \
  samples/toolkit/modern/KALSCOPE.DEM

cargo run --quiet --bin actionc-listing-quality -- \
  target/kalscope-listing-audit-20260726/kalscope/classic.listing \
  > target/kalscope-listing-audit-20260726/kalscope/classic.quality

cargo run --quiet --bin actionc-listing-quality -- \
  target/kalscope-listing-audit-20260726/kalscope/mir6502.listing \
  > target/kalscope-listing-audit-20260726/kalscope/mir6502.quality

ACTIONC_MIR6502_PEEPHOLES=sites \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern \
    --backend mir6502 \
    --emit-load \
    samples/toolkit/modern/KALSCOPE.DEM \
    > /dev/null \
    2> target/kalscope-listing-audit-20260726/kalscope/mir6502.peepholes
```

Baseline load hashes:

| Backend | SHA-256 |
| --- | --- |
| modern/classic | `a7b894d329e03db11e93ff8c07292d9dd5d8d35e100e4b1dbf13decd44fc70c6` |
| modern/MIR6502 | `6521a8ebc302cb49c2a21112a7a1c7f5fee984834e6a2f612d3357a4688e043b` |

## Baseline Measurements

| Metric | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| XEX bytes | 3,318 | 3,683 | +365 |
| Recognized instruction bytes | 2,573 | 2,920 | +347 |
| Data and inline machine bytes | 733 | 751 | +18 |
| Recognized instructions | 1,117 | 1,259 | +142 |
| `LDA` | 337 | 433 | +96 |
| `STA` | 294 | 368 | +74 |
| `LDA` + `STA` instruction share | 56.5% | 63.6% | +7.1 points |
| RAM spill cells | 0 | 15 logical / 17 allocated | +17 allocated |

MIR6502 has 15 referenced RAM spill IDs and 118 RAM spill accesses: 67 loads
and 51 stores. It also uses 33 physical zero-page homes with 152 accesses.

### Routine concentration

The routine figures below are recognized instruction bytes. Classic entry
addresses exclude its inline local-data prefixes.

| Routine | Modern/classic | Modern/MIR6502 | Difference |
| --- | ---: | ---: | ---: |
| `InitP` | 169 | 169 | 0 |
| `InitGr8` | 709 | 1,002 | +293 |
| `Plot8` | 94 | 82 | -12 |
| `Erase8` | 94 | 82 | -12 |
| `GenP` | 389 | 441 | +52 |
| `GenE` | 385 | 437 | +52 |
| `GetParam` | 265 | 266 | +1 |
| `Params` | 160 | 172 | +12 |
| `Kal3` | 311 | 269 | -42 |

`InitGr8`, `GenP`, and `GenE` account for 397 gross excess instruction bytes.
The smaller `Plot8`, `Erase8`, and `Kal3` offset 66 bytes of that excess.

## Finding 1: Committed Word Updates Keep Transient Result Homes

`InitGr8` repeatedly lowers:

```text
load dl
updated = dl + 1
store dl = updated
use updated as the next indirect pointer
```

After byte expansion this becomes a two-lane add into a RAM spill pair, a copy
from that pair back to `dl`, and another copy from the pair into `$AC/$AD`.
Once the store to `dl` has committed, the spill pair and `dl` contain the same
value. The next use can read `dl`; the existing word-update selector can then
turn the add/store into `INC low; BNE; INC high`.

Telemetry observes 18 `staged-word-store-forward` blocks over repeated
fixed-point rounds, representing nine unique sites: seven in `InitGr8` and one
each in `GenP` and `GenE`. They are rejected as
`home-definition-live` because the transient pair is still read after the
committing store.

This is the largest opportunity. It should remove most of `InitGr8`'s 13 RAM
spill homes and 86 spill accesses.

## Finding 2: Word Bitwise Consumers Restage Ordinary RHS Lanes

The eight feedback expressions across `GenP` and `GenE` have the form:

```text
destination = (destination + increment) XOR increment
```

The add result is already in a paired zero-page home. MIR6502 nevertheless
copies both lanes of the ordinary `increment` global into another zero-page
pair before applying `EOR`. Classic applies `EOR absolute` directly.

Allowing `AND`, `OR`, and `XOR` word-chain consumers to use proven ordinary
direct-memory lanes should remove two unnecessary load/store pairs per site.
The rule must retain the existing volatility, alias, absolute-memory, and
fixed-scratch exclusions.

## Finding 3: Pure A/X Byte Calls Preserve the First Result in RAM

`GenP` and `GenE` each make six two-byte calls to `Plot8` or `Erase8`. Both
arguments are pure byte arithmetic over ordinary globals.

MIR6502 evaluates the A argument first, stores it in a RAM spill, prepares the
X argument, then reloads A. The first two sites also spill the X value. Across
the two routines these call clusters account for four RAM spill cells and 32
absolute spill accesses.

When both expression trees are pure, independent, and use only stable ordinary
memory, the backend may evaluate the X expression first, transfer it to X,
then evaluate the A expression. Calls, indirect reads, absolute/hardware
memory, volatile storage, shared scratch, or an expression that writes X must
retain source-order staging.

## Finding 4: Indirect Stores Materialize Direct Values Too Early

`InitGr8` contains byte projections such as:

```text
dl^ = next & $FF
dl^ = next RSH 8
```

The value is loaded into A and spilled before `$AC/$AD` is prepared, then
reloaded for the store. At least five binary-to-indirect-store and four
move-to-indirect-store lanes have this shape.

For stable direct memory and simple low/high byte projections, prepare the
destination pointer first and load the value last. The rewrite must use the
shared alias/effect queries and reject any pointer preparation that can modify
the value source.

## Finding 5: Shift Helpers Have Trusted Narrow Scratch Effects

The final `rsh3` initialization loop copies its word index into RAM before
calling `r_Rsh`, then rematerializes the indexed destination. The repository's
trusted runtime effect map documents that `r_Lsh` and `r_Rsh` write `$85` but
do not write `$AC/$AD`.

MIR6502 currently marks every runtime helper as an opaque unknown-memory
barrier. Encoding the already-documented helper-specific read/write ranges can
allow an indexed destination in `$AC/$AD` to survive a shift helper. This must
be implemented as a runtime-helper contract, not inferred from KALSCOPE.

## Implementation Slices

### Slice 0: Coverage and baseline contracts

Status: complete.

- Add a real-source KALSCOPE quality test. Missing source is a failure.
- Assert baseline selectors and cap the current 3,683-byte output.
- Extend VM coverage with a compact general fixture for:
  - private pointer update followed by indirect stores;
  - word add/XOR feedback;
  - pure two-byte arithmetic call arguments;
  - low/high word projections stored indirectly;
  - indexed byte stores fed by word shifts.
- Keep the existing KALSCOPE machine-block and symbolic-ZP contract gate.

This slice must be code-size neutral.

### Slice 1: Forward committed word values after stores

Status: complete.

- Extend the shared post-home staged-word rewrite to follow a store into an
  ordinary direct word home.
- Retarget exact later reads of the transient pair to the committed home until
  the last use.
- Require same-block dominance, exact two-lane identity, no intervening alias
  write, no unknown effect, and no address-taking.
- Reject absolute, hardware, fixed-ABI, and indirect destinations.
- Let the existing direct word mutation selector fold `+1`/`-1` survivors.
- Record candidates, selections, and blocker reasons.

Result:

- Added exact committed-home forwarding plus a post-placement
  staged-source `INC`/`DEC` selector, both guarded by routine-level home
  liveness, physical alias checks, stable ordinary storage, and flag/A
  observability.
- KALSCOPE selects five committed forwards and five staged word updates.
- `InitGr8` now contains nine `INC.W dl` forms.
- MIR6502 output fell from 3,683 to 3,581 bytes, reducing the classic deficit
  from 365 to 263 bytes.

### Slice 2: Feed direct memory into word bitwise consumers

Status: complete.

- Extend the existing word-chain placement machinery to `AND`, `OR`, and
  `XOR`.
- Permit each RHS lane to remain a stable ordinary direct-memory operand.
- Preserve lane order and reject overlap with the result, volatile/absolute
  memory, fixed scratch, indirect/indexed values, and live carry/flag demands.
- Add positive and rejection tests plus exact KALSCOPE selector counts.

Result:

- Added an alias-aware post-home `AND`/`OR`/`XOR` RHS placement rewrite that
  keeps stable ordinary word lanes in direct memory.
- The rewrite rejects absolute memory and any physical overlap among the
  source, staging, and result pairs.
- KALSCOPE selects eight word-bitwise RHS placements and emits sixteen direct
  global `EOR` operands.
- MIR6502 output fell from 3,581 to 3,517 bytes, reducing the classic deficit
  from 263 to 199 bytes.

### Slice 3: Schedule pure two-byte Action call expressions

Status: complete.

- Recognize calls with byte arguments assigned to A and X.
- Prove both expressions pure, independent, and stable.
- Emit the X expression first, move its result to X, then emit the A
  expression.
- Keep original staging for calls, machine blocks, indirect/absolute reads,
  aliasing storage, X-using expressions, and incomplete proofs.
- Add VM boundary cases and exact KALSCOPE call-site counts.

Implemented coverage:

- `kalscope_codegen_patterns.act` isolates the pointer-update, word-bitwise,
  pure-call-argument, indirect-projection-store, and indexed-shift patterns.
- Its classic and MIR6502 binaries must produce the same byte contract at
  `$0600..$060A` and the `$A5` completion signature at `$0610`.
- `mir6502_kalscope_quality` locks the current KALSCOPE baseline and the twelve
  pure two-byte call sites without constraining future instruction selection.

Result:

- Extended the existing pre-home call-expression selector to retain pure
  left-linear byte `ADD`/`SUB` trees until their A/X destinations are known.
- The selector schedules the X tree first, transfers it to X, then evaluates
  the A tree without a transient home.
- Stable compiler storage and zero-page RAM are accepted through the shared
  pure-read reordering query. Calls, indirect reads, high absolute addresses,
  hardware, and other unstable leaves remain rejected.
- KALSCOPE selects twelve pure A/X schedules and removes the two remaining
  call-cluster RAM spill pairs.
- MIR6502 output fell from 3,517 to 3,419 bytes, reducing the classic deficit
  from 199 to 101 bytes.

### Slice 4: Materialize direct indirect-store values late

Status: complete.

- Extend destination-aware store selection to low/high projections of stable
  direct words and ordinary direct bytes.
- Prepare `$AC/$AD` before loading A when the shared effects and alias queries
  prove the source unchanged.
- Remove only the now-dead transient value home.
- Reject unknown memory effects, volatile/absolute sources, aliasing pointer
  homes, and multi-use results.

Result:

- Added a post-home selector for a stable direct byte followed by a byte
  constant transform and a staged indirect store.
- The selector prepares `$AC/$AD` first, then loads and transforms the value
  immediately before the store. It uses the shared pure-read reordering and
  physical fixed-scratch overlap queries.
- Hardware/high-absolute and fixed-pointer aliases remain rejected.
- KALSCOPE selects three late value placements and removes one staging
  store/reload pair at each low-byte projection.
- MIR6502 output fell from 3,419 to 3,401 bytes, reducing the classic deficit
  from 101 to 83 bytes.
- A final post-home follow-up recognizes `CARD RSH 8` byte consumers after
  home assignment. It replaces the runtime shift with the direct high lane
  while preserving the helper's observable `$84 = 8` and `$85 = 0` scratch
  state. Two KALSCOPE sites select this form and feed the existing late
  indirect-store placement.

### Slice 5: Encode trusted runtime-helper scratch effects

Status: complete.

- Replace the single opaque helper effect with helper-specific effects already
  documented in `docs/RUNTIME_HELPER_EFFECTS.md`.
- Represent exact zero-page reads/writes for shift, multiply, divide,
  remainder, and SARGS helpers; preserve unknown destination writes for SARGS.
- Allow indexed-store destination preparation to cross only helpers proven not
  to write its fixed pointer pair.
- Add effect-classification, negative-clobber, and VM tests.

Result:

- Replaced the opaque arithmetic-helper barrier with documented structured
  zero-page read/write regions and an explicit balanced stack effect.
- Runtime-helper operations now carry their implicit A/X and fixed-ZP argument
  homes, preventing liveness from mistaking helper inputs for dead definitions.
- Structured zero-page effects now also expose their exact fixed-ZP home reads
  and writes. The analysis remains conservative about possible compiler-home
  aliases, but definition-sensitive rewrites can distinguish the helper's
  concrete `$84/$85` definitions.
- `Lsh`/`Rsh`, `Mul`, `Div`, and `Mod` use exact scratch ranges. SArgs retains
  unknown reads/writes because MIR's current effect union cannot express its
  exact `$82-$85`/`$A0-$A2` writes together with its unknown destination;
  this deliberately preserves the conservative boundary.
- Added a shared fixed-pointer overlap query and a post-home indexed-store
  placement guarded by the helper's structured writes.
- KALSCOPE selects one helper-indexed-store placement: `$AC/$AD` is prepared
  before `Rsh`, the original loop index is read directly, and the helper result
  is stored without a transient home.
- MIR6502 output fell from 3,401 to 3,383 bytes, reducing the classic deficit
  from 83 to 65 bytes.
- The exact scratch-home model also made two additional committed pointer
  updates definition-safe. Together with the two direct high-lane projections,
  the final output fell to 3,319 bytes.

### Slice 6: Final audit and stop decision

Status: complete.

Regenerate classic and MIR6502 listings, maps, loads, quality reports,
pre/materialized MIR, spill reports, and site telemetry. Record:

- XEX, instruction, and data bytes;
- per-routine sizes;
- `LDA`/`STA` counts and shares;
- RAM/ZP homes and accesses;
- selected and blocked counts for every new rewrite;
- branch-over-JMP and tail forms;
- the complete modern/MIR6502 Toolkit batch.

Stop unless a remaining KALSCOPE deficit is at least eight bytes, appears at
two general sites or one demonstrably hot loop site, and can be removed using
the shared proofs without weakening effect conservatism.

## Final Audit

Artifacts:

```text
target/kalscope-final-audit-20260726/kalscope/
target/kalscope-toolkit-final/modern-mir6502/
```

Final load hashes:

| Backend | SHA-256 |
| --- | --- |
| modern/classic | `a7b894d329e03db11e93ff8c07292d9dd5d8d35e100e4b1dbf13decd44fc70c6` |
| modern/MIR6502 | `072c3877088cf9d2d274b5b055c430340d09626d86e41201e65ede8bb21b8a3a` |

### Whole-program result

| Metric | Baseline MIR6502 | Final MIR6502 | Modern/classic | Final difference |
| --- | ---: | ---: | ---: | ---: |
| XEX bytes | 3,683 | 3,319 | 3,318 | +1 |
| Recognized instruction bytes | 2,920 | 2,566 | 2,573 | -7 |
| Data and inline machine bytes | 751 | 741 | 733 | +8 |
| Recognized instructions | 1,259 | 1,124 | 1,117 | +7 |
| `LDA` | 433 | 371 | 337 | +34 |
| `STA` | 368 | 302 | 294 | +8 |
| `LDA` + `STA` instruction share | 63.6% | 59.9% | 56.5% | +3.4 points |
| RAM spill homes | 15 IDs / 17 slots | 7 IDs / 7 slots | 0 | +7 |
| RAM spill accesses | 118 | 34 | 0 | +34 |
| Virtual-ZP homes | 33 | 23 | n/a | n/a |
| Virtual-ZP accesses | 152 | 114 | n/a | n/a |
| Branch-over-`JMP` forms | n/a | 0 | 4 | -4 |
| Tail `JSR; RTS` forms | n/a | 0 | 2 | -2 |

MIR6502 removed 364 bytes, or 9.9%, from its KALSCOPE baseline. Its final
recognized instruction stream is seven bytes smaller than classic; the
one-byte XEX deficit is the net result of eight additional data/home bytes.

### Routine result

The classic `InitGr8` figure uses the routine-entry convention from the
baseline audit; the full-program instruction totals above are authoritative.

| Routine | Baseline MIR6502 | Final MIR6502 | Modern/classic | Final difference |
| --- | ---: | ---: | ---: | ---: |
| `InitP` | 169 | 169 | 169 | 0 |
| `InitGr8` | 1,002 | 806 | 709 | +97 |
| `Plot8` | 82 | 82 | 94 | -12 |
| `Erase8` | 82 | 82 | 94 | -12 |
| `GenP` | 441 | 362 | 389 | -27 |
| `GenE` | 437 | 358 | 385 | -27 |
| `GetParam` | 266 | 266 | 265 | +1 |
| `Params` | 172 | 172 | 160 | +12 |
| `Kal3` | 269 | 269 | 311 | -42 |

### Final selector counts

| Selector | Selections |
| --- | ---: |
| committed word-store forwarding | 7 |
| staged word `INC`/`DEC` | 7 |
| direct word bitwise RHS | 8 |
| pure A/X byte-call scheduling | 12 |
| transformed direct indirect store | 3 |
| direct indirect store | 2 |
| word `RSH 8` high projection | 2 |
| helper-crossing indexed store | 1 |

The complete modern/MIR6502 Toolkit batch passed 20/20 programs and totals
43,222 bytes.

### Stop decision

Stop. The remaining whole-program deficit is one byte, below the eight-byte
threshold. `InitGr8` still contains seven RAM homes and one unfused high-word
projection, but pursuing those forms solely for KALSCOPE would add analysis
surface without a meaningful program-size result. Future work should be driven
by a repeated pattern in another audited program, not this residual byte.

### Validation result

- `cargo test`: passed.
- KALSCOPE backend-contract VM gate: passed.
- KALSCOPE codegen-pattern VM gate: passed.
- TN stability gate: passed.
- ALLOCATE runtime gate: passed.
- SORT runtime gate: passed.
- CIRCLE INT runtime gate: passed.
- modern/MIR6502 Toolkit batch: 20 successes, 0 failures.

## Validation

After every behavior-changing slice run:

```sh
cargo test --lib
cargo test --test mir6502_kalscope_quality
fixtures/runtime/run-kalscope-contracts-vm.sh
fixtures/runtime/run-kalscope-codegen-patterns-vm.sh

tools/compare-codegen.sh \
  --profile modern \
  --out-dir target/kalscope-slice-audit \
  --no-diffs \
  samples/toolkit/modern/KALSCOPE.DEM
```

Before accepting the final slice run:

```sh
cargo test
cargo test --test compatibility kalscope_backend_contract_runtime_check -- --ignored
cargo test --test compatibility kalscope_codegen_patterns_runtime_check -- --ignored
cargo test --test compatibility tn_stability_check -- --ignored
cargo test --test compatibility allocate_runtime_check -- --ignored
cargo test --test compatibility sort_runtime_check -- --ignored
cargo test --test compatibility circle_int_math_runtime_check -- --ignored

surveys/toolkit/compile-toolkit-batch.sh \
  --preset modern-mir6502 \
  --output-dir target/kalscope-toolkit-final
```

No NIR change is planned. If a slice changes NIR, semantic lowering, the NIR
verifier, or the NIR printer, also run the repository-required NIR fixture and
sweep gates.
