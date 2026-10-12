# MIR6502 comparison loop qualification

The [implementation plan](../comparison-loop-pointer-reuse-plan.md) separates
pointer lifetime proofs, retained bases, comparison fusion and Y induction.

## Stage 0 baseline

[stage0.json](stage0.json) pins the compiler executable, source revision,
four current TN/TNDBG images, generic fixture hash, direct comparison vectors
and exclusive routine measurements. Large images, listings and typed/selected
MIR dumps are retained under `target/tn-comparison-loops/stage0`.

Both MIR roots contain a 246-byte `MyDosAfter` with zero spills, four parameter
bytes and three local bytes. Both reversed 64-entry sorts take 534,512 cycles
including the wrapper. The existing sort fixture contains `FILEnnnn` names
with common prefixes and an independently ordered host oracle.

The generic fixture and VM target cover first/middle/last and every other
mismatch position, equal and identical pointers, unsigned byte boundaries,
page crossings, a displacement-239 field, exact pointee read order, guards and
observable character/index values. They run MIR6502 cartridge and standalone
with LF and CRLF input. Classic is an independent source/ABI control.

Typed MIR has ordinary local induction and capture cells. The comparison body
loads each pointer, adds its embedded-field displacement, loads the common
index, then stores each character into a local before testing inequality.
The mismatch successor reloads those same characters for ordering. The
existing contiguous dual-indirect matcher cannot eliminate these capture
stores or their successor uses. The current counted-loop contract also does
not describe this body with its early-return exit.

Candidate discovery belongs in the pre-home canonicalization group after CFG
cleanup and block-argument lowering, before comparison and address expansion.
This group already has provisional physical layout and reserved pointer pairs.
Final ownership checks must be repeated after allocation/coloring and emission
layout. A companion comparison-loop fact will preserve the existing loop
selectors' narrower contracts.

## Stage 1 facts

The companion analysis records the unique preheader, header, body, latch and
both exits with a snapshot generation. Complete pointer lanes must have one
exclusive defining chain from ordinary parameter/local storage. Captures,
index and base cells are distinct; only the original ordered reads, captures
and exact unsigned inequality are admitted. Foreign entries, zero-trip or
wrapping bounds, source writes, effects, incomplete lanes and extra temp uses
retain the generic path. This stage changes no emitted operations.

## Stage 2 retained pointers (opt-in)

Both runtime/newline paths pass with the feature disabled and enabled. The
single-loop fixtures retain two bases and execute four pointer-pair writes per
call, independent of equal-prefix length. `Equal` takes 648 cycles for equal
11-byte inputs versus 1,020 before selection; first-byte mismatch takes 129
versus 125. Necessary captures and observable index values remain intact.

The two-loop `After` candidate grows by two instruction bytes in this slice
and retains its complete fallback. Final routine, home and image budgets are
checked separately; a bounded partial-batch repair lets smaller routines pass.
Both pairs use the existing `$AA–$AF` compiler window. The selected schedule
adds no virtual zero-page allocation outside that window. The live-Y verifier
checks complete first-entry initialization through the CFG.

## Stage 3 shared comparisons (opt-in)

The generic VM matrix passes with features off, retained pointers and fused
comparisons. Selected fixtures execute one ordered indirect LDA/CMP per
visited byte pair; their mismatch ordering reuses C without rereading memory.
`After` now passes the final size budget, including both original loops. Equal
14-byte inputs take 627 cycles versus 1,275; first-byte mismatch takes 117
versus 136. `Observed` retains its captures and benefits from pointer retention.

Capture removal requires path-sensitive private-cell use facts, no address
exposure and a repeated final check for separately named absolute aliases.
Complete temp uses and exact ordering operands are checked independently.
Unit coverage includes stale transactions, first-entry base initialization,
partial pair clobbers, observable captures and address exposure.

## Stage 4 bounded Y induction (opt-in)

The four-way feature matrix passes in both runtimes and newline forms. The
selected equal-prefix latch contains INY/CPY and branches directly to the
ordered comparison. Proven nonempty entry skips the first guard; zero-trip
and unsupported intervals retain fallback. The final Y value is bounded by
255. Observable induction keeps its original stores, guard and increments.

Equal 14-byte `After` takes 391 cycles versus 1,275; first-byte mismatch takes
100 versus 136. Equal 11-byte `Equal` takes 304 versus 1,020. VM checks count
the expected Y advances as well as read order, guards and returned values.
All three transaction/fusion/carrier unit tests pass, including extra header
temp users and a first entry missing Y initialization.

## Stage 5 qualification and enablement

`Mir6502Config::optimized()` enables retained pointers, shared comparisons and
bounded Y induction. Default and explicit feature-off configurations retain
fallback. The [final report](stage5.json) pins the qualified compiler binary,
source hashes, images, four-way direct vectors and exclusive application
measurements. Production TN source is unchanged.

| Measurement | Baseline | Qualified |
| --- | ---: | ---: |
| TN MIR load-file bytes | 16,764 | 16,705 |
| TNDBG MIR load-file bytes | 20,501 | 20,442 |
| `MyDosAfter` instruction bytes, both roots | 246 | 187 |
| Reversed 64-entry sort cycles, both roots | 534,512 | 343,356 |
| Exclusive `MyDosAfter` cycles in that sort | 305,325 | 114,169 |
| Executed pointer-scratch writes in that routine | 13,908 | 2,624 |
| Executed parameter operand accesses | 15,258 | 4,798 |
| Executed local operand accesses | 25,642 | 1,838 |
| Executed indirect character reads | 6,954 | 6,954 |

The sort takes 35.8% fewer cycles. Exclusive comparison cycles fall 62.6%,
and pointer-scratch writes fall 81.1%. These are workload measurements, not
universal speedup claims. Static homes remain zero spill, four parameter and
three local bytes. Both roots use the same existing `$AA–$AF` compiler window;
the final gate permits no added allocation outside it. Classic images are
byte-identical to Stage 0 at 18,090 and 21,579 bytes. Full LF and CRLF source
trees produce identical images in both modes.

The generic equal 14-byte `After` takes 391 cycles versus 1,275, and its
first-byte mismatch takes 100 versus 136. Observable captures/index retain
their stores: `Observed` improves from 1,033 to 671 cycles for equal inputs,
but first-byte mismatch rises from 160 to 165. Pointer-only `Equal` also costs
four extra cycles on first-byte mismatch. Final size budgets can reject an
entire two-loop pointer-only candidate; comparison fusion enables that fixture
to pass without code growth.

Qualification passes the full MIR6502 unit suite (1,057 tests, two existing
ignores), eleven integration targets (96 tests), the 169-fixture emission
sweep and the full VM/runtime suite. The focused comparison matrix runs four
feature variants in cartridge and standalone runtimes with LF and CRLF.
Additional VM cases cover all unsigned ordering directions, partially
overlapping records, a 255-byte window, zero trips, guarded 16-bit wrap,
exposed capture aliases and source ownership of `$AA`. Exposed frames and
local alias metadata block capture/index removal; malformed live-Y consumers
and initialization arriving only from a backedge fail verification.

Reproduce the image and listing census with:

```sh
python3 surveys/tn/comparison-loops/measure.py \
  --compiler target/tn-comparison-loops/final/actionc-qualified \
  --out-dir target/tn-comparison-loops/recheck
```

Use `cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test
comparison_loops -- --nocapture` for the four-way vectors. For each root, set
`ACTIONC_TN_PROFILE_SOURCE=TN` or `TNDBG` and
`ACTIONC_TN_PROFILE_ROUTINES=1`, then run the `tn_size_profile` target with
`--nocapture`. The CSV columns are retained in the report. Build with the
task environment documented below if recreating the qualified binary:

```sh
export CARGO_TARGET_DIR=target/tn-comparison-loops/build
export CARGO_INCREMENTAL=0
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0
export CARGO_PROFILE_DEV_OPT_LEVEL=1 CARGO_PROFILE_TEST_OPT_LEVEL=1
cargo build --bin actionc --bin actionc-mir6502-sweep
```

The disposable Rust build directory is removed after qualification. Frozen
Stage 0/final compilers, images, listings and check logs remain in `target`;
the source fixture and compact measurement reports are tracked here.
