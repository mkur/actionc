# Address-value implementation qualification

The [implementation plan](../address-values-implementation-plan.md) owns the
stages; the [audit](../output-audit-2026-10-12/README.md) supplies the frozen
full-root measurements and cycle profiles.

## Stage 0

[stage0.json](stage0.json) pins the qualified compiler SHA, both generic
cartridge listings' instruction/data and home census, and the four root images.
The source layout and compiler are unchanged. New
[known-address cases](../reproducers/known-record-address.act) retain complete
pointer assignments while reading/writing fixed and compiler-placed records.

Current MIR instruction bytes are 11 for direct CARD field access, 32 through
a known-address local, 45 for a local-pointer field store, and 9 for the returned
constant field address. The dynamic stride-20 local read remains 104 bytes;
the returned address remains 76. These are baseline observations, not CI size
assertions.

The `address_values` and `tn_codegen_reproducers` VM targets passed (2 and 3
tests respectively). They execute classic/MIR, cartridge/standalone and actual
LF/CRLF sources. The new target checks both return lanes, full source-local
pointer contents, genuinely changed word stores, and surrounding guard bytes.
Existing `record_consumer_boundaries` owns effectful indexes and two record
layouts; proof/alias rejection coverage is extended with its selector stages.

Development uses a single isolated, debug-info-free target directory:

```sh
CARGO_TARGET_DIR="$PWD/target/tn-address-values/build" CARGO_INCREMENTAL=0 \
  CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=1 \
  cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test address_values --test tn_codegen_reproducers -j2 -- --nocapture
```

Listings and logs are retained under `target/tn-address-values/stage0`. The
historical audit collector remains pinned to the pre-implementation compiler;
later stages write separate reports rather than replacing its measurements.

## Stage 1

The complete word `ComputeAddress` operation and its conservative full-width
fallback are implemented. Same-block provenance follows exclusive word copies,
constant displacements and constant-stride scaling; it retains originating
loads/calls and records input evaluation sites. Reaching-pointer-store queries
reject nonlocal cells, mutations and call/unknown-write barriers. Selection is
still disabled.

Six focused MIR tests pass: complete-word/phase verification, operation-family
effects, three provenance checks and actual fallback execution. The execution
check covers both materialization profiles/runtimes, strides 1/2/20/255/256/65535,
offsets 0/38/255/256, unaligned bases and full CARD wrap.
[stage1.json](stage1.json) confirms all four full-root images are byte-identical
to Stage 0. SemIR/NIR contracts and snapshots are unchanged.

## Stage 2

Optimized MIR forwards a known storage address from an exact complete local
pointer store to its same-block field consumer. The store and source evaluation
remain. Typed direct operands resolve at final layout; descriptor addresses and
fixed pointer cells are excluded. The transaction driver independently checks
the unchanged prefix, retained word store and exact resulting field identity.
Calls, any intervening write and one-lane mutation invalidate the fact.

| Generic cartridge routine | Stage 0 instruction bytes | Stage 2 | Local bytes |
| --- | ---: | ---: | ---: |
| ReadLocal / ReadOrdinary | 32 | 21 | 2 |
| WriteLocal | 45 | 26 | 2 |
| AddressZero | 63 | 15 | 0 |
| AddressField | 18 | 18 | 0 |

Both known local reads now have direct field operands and no indirect-Y access;
the source local still receives both pointer bytes. Direct fixed field reads
remain 6/11 bytes. `AddressZero` no longer calls multiply and has no spills.
The algebraic `AddressExpanded` comparison remains 222 bytes; the selector
currently requires an exclusive address flow to a complete word store.

[stage2.json](stage2.json) retains cartridge/standalone measurements and actual
LF/CRLF image equality. Three selector proof tests, the five generic VM tests,
and the four affected integrations pass (call ABI, MIR fixtures, materialization
gaps and rewrite inventory). No NIR or MIR-lowering snapshots changed.

Reproduce generic measurements with
`python3 surveys/tn/measure-address-values.py --compiler <actionc> --out-dir <target-directory> --crlf`.
The collector checks both runtimes and can add full TN/TNDBG roots with `--roots`.

## Stage 3

Indexed complete addresses use the existing private full-pair constructor when
final code plus homes and the whole image meet the fallback's size budget.
BYTE indexes can remain in Y across a preserving constructor; saved-home
deadness and physical input/alias checks protect that reuse. CARD indexes retain
both lanes. Strides above 255 use full-width arithmetic.

The generic stride-20 `Address` changes from 76 code + 4 spill + 3 parameter
bytes to 77 code + 0 spill + 3 parameter bytes. Its multiply call disappears;
measured cartridge execution, including the wrapper, is 151 cycles at every
tested BYTE index, previously 246–459. `ReadViaLocal` still uses its fallback
at this stage: constructing and then reloading its source pointer is too large.

[stage3.json](stage3.json) records TN/TNDBG MIR load sizes of 16,802/20,539,
five bytes below their frozen baselines. Both classic images are unchanged;
all four roots and both generic runtime fixtures match their actual CRLF builds.
The full MIR unit set passes (1,035 tests; two existing ignored tests), as do
the address, record-boundary, generic reproducer and TN CPU-profile VM targets.
Forced trial execution also checks rejected size candidates, full CARD indexes,
page crossings and wrap in both runtimes.

## Stage 4

An indexed address stored to an ordinary source pointer can supply its next
field consumer through the complete prepared pair. The transaction retains
both source pointer bytes, all intervening direct reads and exact field lanes.
Y-window rebasing happens after the pointer assignment. Writes/calls and
pointer mutation stop reuse. Exclusive CARD index loads can feed the constructor
from ordinary parameter/local storage without a private capture; absolute
backing and source/scratch collisions reject that deferral.

The stride-20 `ReadViaLocal` now has 101 code + 0 spill + 4 parameter + 2 local
bytes, previously 104 + 2 + 4 + 2. It has no multiply call and never reloads
the source pointer for its field. Measured cartridge execution is 188 cycles
for all tested CARD indexes, previously 291–604. Direct consumption remains
85 bytes/166 cycles; retaining the source assignment still has a cost.

[stage4.json](stage4.json) retains both runtime measurements and four-root
LF/CRLF equality. Root sizes match Stage 3. The MIR unit set passes (1,037;
two existing ignored tests), and ten tests across the four affected VM targets
pass. New execution cases check retained pointer bytes, byte/word fields at
offsets 0/38/254/255/256, independent layouts, effectful indexes, aliasing writes,
one-lane mutation, intervening calls and a pointer referring to its own cell.

## Stage 5: integrated qualification

All stages are complete. New index and address-value read deferral preserves
input snapshots across hardware, absolute and unknown-pointer reads, in
addition to existing write/call barriers. Selection remains optimized-MIR-only;
the conservative profile and every rejected candidate retain executable
full-width arithmetic. No SemIR/NIR contract, application source or ABI changed.

[stage5.json](stage5.json) retains the final compiler hash, both runtimes,
four-root census, LF/CRLF comparisons and the independently measured second
layout baseline. Instruction bytes and emitted spill bytes are separate:

| Layout / routine | Frozen instruction bytes | Final instruction bytes | Frozen → final spill bytes |
| --- | ---: | ---: | ---: |
| Stride 20, displacement 38: `Address` | 76 | 77 | 4 → 0 |
| Stride 20, displacement 38: `ReadViaLocal` | 104 | 101 | 2 → 0 |
| Stride 20, displacement 38: `ReadDirect` | 85 | 85 | 0 → 0 |
| Stride 20, displacement 38: `AddressZero` | 63 | 15 | 4 → 0 |
| Stride 2, displacement 256: `Address` | 54 | 44 | 0 → 0 |
| Stride 2, displacement 256: `ReadViaLocal` | 95 | 68 | 0 → 0 |
| Stride 2, displacement 256: `ReadDirect` | 60 | 60 | 0 → 0 |

The [second fixture](../reproducers/escaping-record-address-wide.act) has a
different stride and a full-word embedded displacement. Both local forms
retain two source pointer bytes and four parameter bytes. Returned BYTE-index
addresses retain three parameter bytes. Thus the stride-20 return grows by one
instruction byte but saves four spill bytes: its code-plus-home cost is 83 → 80.
The local form costs 112 → 107. Both admitted stride-20 forms have no multiply
call or spill home; direct field consumption stays unchanged.

| Cartridge root | Frozen load-file bytes | Final load-file bytes |
| --- | ---: | ---: |
| TN classic | 18,095 | 18,095 |
| TN MIR6502 | 16,807 | 16,802 |
| TNDBG classic | 21,586 | 21,586 |
| TNDBG MIR6502 | 20,544 | 20,539 |

Both classic image hashes match Stage 0 exactly. Final images include branch
relaxation, retained helpers and shared homes; routine savings are not added
together to predict image sizes. The general multiply helper remains available
to other consumers. Every measured root and generic cartridge/standalone image
matches its actual CRLF-source build.

[workloads.csv](workloads.csv) and [profiles.csv](profiles.csv) retain final
classic/MIR vectors and independent per-routine attribution for TN and TNDBG.
Their cycle and spill sums reconcile for every vector. Repeated identical
publish runs are represented once. Against the frozen audit, both MIR roots'
64-row reversed sort changes as follows:

| Measurement | Frozen | Final |
| --- | ---: | ---: |
| Whole sort cycles, including wrapper | 805,176 | 534,512 |
| Executed emitted spill operands | 15,112 | 1,616 |
| `BatchItem` body plus its multiply helper, cycles | 399,840 | 129,176 |
| `BatchItem` multiply calls | 964 | 0 |
| `BatchItem` spill operands | 13,496 | 0 |

This is **33.6% fewer sort cycles and 89.3% fewer spill operands**. The old
`BatchItem` cost includes 104,112 body cycles and 295,728 resident-helper cycles;
the final body needs no helper. Fixed pointer-scratch writes increase from zero
to 7,712 over its 964 invocations, so spill removal does not mean zero private
traffic. Cycles include the fixed wrapper, resident ROM and substituted service
returns, excluding real I/O latency and ANTIC contention. Spill counts cover
executed generated RAM operands to named homes; they exclude stack, resident
ROM operands and pointer-fetch bus bytes. Read-modify-write instructions count
their semantic read and write accesses.

Qualification passed 1,039 MIR6502 library tests (two existing ignored), 96
tests in all eleven MIR6502 integration targets, and 386 tests in the full
`tools/vm-runtime-tests` crate. This includes runtime/ABI, TN provider/directory
acceptance, field boundaries, both complete return lanes, effectful indexes,
retained pointer contents, aliases and mutation. Raw-memory execution checks
the conservative fallback and forced constructor trials in both runtimes,
including unrestricted CARD indexes and a word field crossing `$FFFF` to zero.
Forced trials test semantics of size-rejected forms; they do not bypass the
production gate. MIR fixtures pass unchanged; no NIR snapshots changed.

Remaining conservative cases are explicit. Cross-block propagation, multiple
address users, uncertain effects/aliases, scratch collisions, live flags and
unproved lane uses retain their original path. Stride 255 expansion may be
rejected for size; strides above 255 retain full-word arithmetic without
narrowing. Individual CARD-index returns can also lose the size comparison.
`AddressExpanded` remains 222 bytes with its multiply helper and four spill
bytes. In the second layout, early power-of-two lowering hides the zero-index
arithmetic shape from this selector: `AddressZero` remains 48 bytes versus 18
for `AddressField`. Broader arithmetic canonicalization is follow-up work.

Reproduce the image census using the maintained collector:

```sh
python3 surveys/tn/measure-address-values.py \
  --compiler target/tn-address-values/final/actionc-qualified \
  --out-dir target/tn-address-values/recheck --roots --crlf
```

Reproduce CPU attribution with the development target/profile variables above
and `ACTIONC_TN_PROFILE_ROUTINES=1 ACTIONC_TN_PROFILE_SOURCE=TN` (then `TNDBG`),
running `cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml
--test tn_size_profile -j2 -- --nocapture`. Conservative materialization is
qualified through the library API, not a 6502 CLI optimization switch. Final
compiler, listings, images and logs stay under `target/tn-address-values`; the
temporary build cache can be removed without deleting those artifacts or the
frozen historical baselines.
