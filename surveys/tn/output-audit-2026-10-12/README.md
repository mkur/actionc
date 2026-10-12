# Current TN/TNDBG compiler-output audit

The next compiler priority is **indexed address values that are returned or
stored in source locals**. Existing record load/store selection helps direct
consumers, but these other uses still take the general arithmetic path. This
is the largest demonstrated execution cost in sorting and explains why the
generic `ReadElement` remained 104 bytes after the previous compiler work.

Two smaller directions have independently measured potential: bounded byte
arithmetic in bitmap operations, and argument captures that survive a pure
predicate's control flow. There is no measured justification here for deleting
provider validation or changing the directory layout.

This audit changes diagnostics and fixtures. Production TN and compiler output
remain unchanged. The source experiments below use isolated copies.

## Pinned inputs and measurements

The audited application revision is `02d79b01`; compiler source last changed at
`1d67c984`. The retained Stage 7 compiler builds cartridge images at `$2C00`.
All four images match the [final qualification](../size-reduction/final.json)
byte for byte. [measurements.json](measurements.json) pins compiler, emitter,
source, fixture and profiler hashes, complete memory measurements, and the
largest routines. The raw measurements are:

- [routines.csv](routines.csv): every routine in all four builds, instruction
  and directive forms, emitted/fixed homes, call sites, pointer scratch stores,
  indirect accesses, ABI operands and named storage operands.
- [profiles.csv](profiles.csv): exclusive executed cycles and operand traffic
  attributed to each routine, including ROM and wrapper buckets.
- [workloads.csv](workloads.csv): total cycles and executed spill operands for
  every measured workload and counterfactual.
- [reproducers.csv](reproducers.csv): generic execution measurements, including
  both backends, both runtimes, and actual LF/CRLF compilation.

| Program/backend | Load-file bytes | Instruction-form bytes | Directive-form bytes |
| --- | ---: | ---: | ---: |
| TN classic | 18,095 | 16,659 | 1,424 |
| TN MIR6502 | 16,807 | 15,389 | 1,406 |
| TNDBG classic | 21,586 | 19,589 | 1,985 |
| TNDBG MIR6502 | 20,544 | 18,550 | 1,982 |

The census reconciles every annotated byte with the XEX, plus its symbolic
two-byte RUNAD vector. Instruction/directive forms describe the listing:
MIR's embedded runtime contains machine instructions written as raw data,
whereas classic lists some of them as instructions. These columns therefore
are not a semantic code/data comparison between backends. XEX headers are
included only in load-file size; deferred arrays are not loaded bytes.

Homes count physical named bytes, with emitted and fixed cells separate.
Categories can alias and must not be added together. Classic's absence of
`spill_` names does not mean it performs no temporary-memory traffic. Static
operands count addressing sites, including two semantic accesses for RMW;
indexed bases are not promises about the actual accessed cell.

The previous qualification's routine census used whole PROC extents. Its
`MyDosParse` 363 and `MyDosRender` 298 bytes include three inline SArgs descriptor
bytes each; their instruction forms are 360 and 295. This is a measurement
clarification, with no image or savings change.

## Size shortlist

| TN MIR routine | Instruction bytes | Emitted spill/parameter/local bytes | JSR sites | Pointer scratch store sites |
| --- | ---: | ---: | ---: | ---: |
| `Copy` | 859 | 3 / 0 / 17 | 40 | 10 |
| `Handle` | 576 | 4 / 0 / 6 | 32 | 0 |
| `TagsExtent` | 521 | 4 / 5 / 11 | 2 | 18 |
| `ChangeTagged` | 501 | 2 / 5 / 10 | 2 | 24 |
| `MyDosRead` | 435 | 2 / 4 / 5 | 9 | 8 |
| `MyDosName` | 376 | 1 / 6 / 4 | 1 | 24 |
| `MyDosParse` | 360 | 0 / 5 / 2 | 1 | 22 |
| `SortBatch` | 314 | 2 / 2 / 11 | 4 | 12 |
| `MyDosRender` | 295 | 1 / 5 / 2 | 5 | 24 |
| `BatchItem` | 76 | 4 / 3 / 0 | 1 | 0 |
| `SetTagged` | 67 | 5 / 5 / 0 | 1 | 0 |

`SetTagged` also tail-jumps to `ChangeTagged`; JSR counts exclude tail jumps.
The zero scratch-store count for `BatchItem` does not make its address
construction cheap: it calls the cartridge's general multiply helper.

TNDBG's largest routines are `Copy` at 1,541 instruction bytes and `Dbg` at
1,452, with 50 and 154 JSR sites respectively. They merit separate copy/debug
command profiles before restructuring; the workloads here do not execute
them. Their 2,993 instruction bytes are not an estimate of removable overhead.

The largest MIR-versus-classic instruction gaps are `TagsExtent` (+89),
`SetTagged` (+33), `Draw` (+22), and `BatchPublish` (+21). These identify
inspection sites, not attainable savings. Several pointer-heavy routines
already favor MIR: `MyDosName` 376 versus 509, `SortBatch` 314 versus 472,
`MyDosParse` 360 versus 465, and `MyDosRender` 295 versus 418.

## Executed costs

The 64-entry reversed sort takes 805,176 MIR cycles and performs 15,112
executed spill operands. Its exclusive cycle attribution is:

| Routine/bucket | Cycles | Spill operands | Pointer scratch stores | Indirect-Y operations |
| --- | ---: | ---: | ---: | ---: |
| `MyDosAfter` | 305,325 | 0 | 13,908 | 6,954 |
| Resident ROM | 295,728 | 0 | outside operand census | 0 |
| `BatchItem` | 104,112 | 13,496 | 0 | 0 |
| `SortBatch` | 99,994 | 1,616 | 1,926 | 1,477 |
| Wrapper | 17 | 0 | 0 | 0 |

`BatchItem` is called 964 times and accounts for 89.3% of sorting's measured
spill operands. Its only ROM call is `MulI`; the other generated sort routines
have no ROM-helper path. Its own cost plus the ROM bucket is 399,840 cycles,
49.7% of the workload. This call-graph attribution makes address construction
the first compiler target. It does not establish that all those cycles can
be removed.

Publishing a 1,088-entry prefix takes 355,209 cycles and only 18 spill operands.
`TagBitInRange` contributes 225,216 exclusive cycles, `TagsRecount` 66,527,
and ROM helpers 62,883. This is an arithmetic/helper cost, with little private
spill traffic.

The new end-of-list vector publishes extent 1,089 with every bit initially set.
It checks the retained full bytes, the partial final byte, the cleared tail,
completion state, counts and both outer guards. It takes 473,247 cycles and
2,276 spill operands. `TagsExtent` now contributes 49,440 cycles and 2,267
spill operands while clearing 375 tail bytes. A large routine can therefore
have very different costs on prefix versus completion paths.

The 16-row draw takes 55,178 cycles; `MyDosRender` contributes 22,448, ROM
8,869, and `Internal` 7,998. Existing MyDOS text conversion is part of that
behavior. No evidence here establishes redundant validation on this path.

TN and TNDBG agree for these measured MIR vectors. Cycles include executed
ROM instructions, the fixed call wrapper and substituted screen/disk service
returns. Operand counts cover generated RAM code and named homes, exclude
pointer-fetch bytes, stack traffic and ROM operands, and count semantic RMW
reads/writes rather than dummy bus writes. They do not measure real device
latency, CIO work or ANTIC DMA. Routine attribution is exclusive, not inclusive.

## Ranked compiler directions and generic evidence

### 1. Address values through locals and returns

The [implementation plan](../address-values-implementation-plan.md) defines
the proof foundations, selection slices and qualification for this direction.

[escaping-record-address.act](../reproducers/escaping-record-address.act)
uses a 20-byte record stride, a 38-byte embedded offset, and BYTE/CARD indexes.
It compares equivalent direct reads, reads through a source pointer local,
and returned addresses. Current MIR cartridge output is:

| Form | Routine bytes | Spill/local bytes | Multiply helper | Cycles including wrapper |
| --- | ---: | ---: | --- | ---: |
| `ReadViaLocal`, CARD index | 104 | 2 / 2 | yes | 291–604 on measured indexes |
| `ReadDirect`, CARD index | 85 | 0 / 0 | no | 166 |
| `Address`, BYTE index | 76 | 4 / 0 | yes | 246–459 |
| `AddressExpanded`, BYTE index | 222 | 4 / 2 | yes | 444 |
| `AddressZero` | 63 | 4 / 0 | yes | 228 |
| `AddressField` | 18 | 0 / 0 | no | 47 |

The direct/local comparison demonstrates 19 routine bytes plus four named
home bytes of potential, and a helper-free strategy already available for
the corresponding memory consumer. It does not justify eliminating an
observable local write or invent a whole-TN byte saving. The zero-index
comparison exposes another 45 routine bytes plus four spill bytes in the
generic pattern; current TN has no matching `@record.array(0)` source site.

The existing [record-consumer rewrite](../../../src/mir6502/rewrite/addresses.rs)
accepts a typed load/store address use and an exclusive multiply/add chain.
Returned values and address writes into locals fall outside that consumer
shape. Extend target strategy for address values using typed MIR facts and
the established word-flow, effect and flag proofs. Preserve source captures,
public results and 16-bit arithmetic, including wide-index wrap. Cost the
final instructions and homes rather than the number of virtual operations.

The algebraic shift/add source expansion is rejected: it grows the fixture
by 146 routine bytes and still calls multiply for its zero-index base.
Production `BatchItem` should stay shared; its previous expansion experiment
cost 337 MIR load bytes and 15 home bytes per root.

### 2. Bounded byte arithmetic

[bounded-bit-mask.act](../reproducers/bounded-bit-mask.act) makes the range
proof explicit with `ordinal & 7`. Despite the BYTE result and bounded count,
`ShiftMask` uses word-shift helper `$B5C0`: 17 routine bytes and 57–145 cycles
across bit positions. An eight-byte inline table uses 15 routine bytes and
42 cycles. The table's bytes must be included in any size comparison.

The isolated TN experiment shares that table across three bitmap-mask sites.
It saves only two load bytes per MIR root, but reduces prefix-publication
cycles by 18.5%. Focus on target selection for bounded byte computations,
unneeded high lanes and the cost of helper calls. Dead lanes still require
use/flag proofs. A compiler-owned table would require constant-data ownership
and cost sharing; the source fixture's mutable global cannot be treated as a
compiler constant without proof.

### 3. Argument capture across pure predicate control flow

[captured-call-arguments.act](../reproducers/captured-call-arguments.act)
compares `Apply(cell,value,setting#0)` with computing that pure predicate first.
The first form needs 70 routine bytes and five spill bytes; the second uses
43 routine bytes, no spills and one extra local byte. Both retain five
parameter bytes. The measured opportunity is 27 routine bytes plus four net
home bytes, with 32 fewer cycles in the generic call.

The corresponding `SetTagged` counterfactual saves 31 MIR load bytes in each
full root. Existing workload timings do not exercise that wrapper, so they
remain unchanged; the generic test supplies its execution evidence. Classic
size increases, making this primarily a MIR carrier/availability issue.

Investigate value availability across the predicate's blocks, using reaching
definitions, dominance and final physical interference facts. Keep canonical
parameter captures, argument order and public ABI storage. Effectful later
arguments, aliasing writes and calls need negative coverage; this pure-source
comparison is not evidence that argument evaluation can generally be reordered.

### Subsequent profiling and proof work

`MyDosAfter` performs repeated pointer setup but no named spilling. Its
305,325 sort cycles make it a candidate for comparison-loop/base reuse once
safe ownership, alias and clobber facts exist. This audit has no validated
alternative with a byte/cycle saving, so it ranks after the three demonstrated
directions. The same restraint applies to `TagsExtent` loop-invariant loads
and tail-clear pointer traffic. Existing bounds, snapshot captures, calls
and pointer writes remain conservative barriers.

Profile copy continuation and debug commands before selecting a strategy for
their large static routines. Retain the 64-entry cache, byte order table,
provider `position`, checked public interfaces and future FujiNet contract.

## Full-root counterfactuals

Positive values below mean load bytes saved from the pinned baseline.

| Isolated source experiment | TN classic | TN MIR | TNDBG classic | TNDBG MIR |
| --- | ---: | ---: | ---: | ---: |
| Shared bounded-mask table | 5 | 2 | 7 | 2 |
| Pure predicate before call capture | -9 | 31 | -4 | 31 |
| Both changes together | -3 | 33 | 0 | 33 |

Classic layout and branch changes make these savings nonadditive. The combined
MIR load sizes are 16,774/20,511. These are measured diagnostic comparisons,
not release-qualified application changes.

| MIR workload | Baseline cycles | Mask/combined cycles | Reduction |
| --- | ---: | ---: | ---: |
| Reversed sort, 64 | 805,176 | 805,176 | 0% |
| Draw, 64 entries / 16 rows | 55,178 | 54,210 | 1.8% |
| Tag all, 64 | 26,294 | 22,418 | 14.7% |
| Toggle one, 64 | 755 | 740 | 2.0% |
| Publish prefix, 1,088 | 355,209 | 289,381 | 18.5% |
| Publish complete, 1,089 | 473,247 | 407,371 | 13.9% |

Executed spill counts are unchanged by these source experiments. The predicate
experiment alone has unchanged workload cycles. Combined TNDBG matches TN for
the measured MIR vectors; the independent mask/predicate profiles use TN.

## Validation and reproduction

Focused validation passed the three tests in `tn_codegen_reproducers` and the
three tests in `tn_size_profile`. Generic fixtures execute classic/MIR6502,
cartridge/standalone, actual LF/CRLF files, unaligned/page-crossing record
bases, BYTE indexes through 255 and CARD indexes through 65,535, full neighbor
guards, all mask positions and pure-call values/flags. Both real roots and the
counterfactual profiles preserve sorting, rows, tag states and bitmap guards.
The default profile CSV is unchanged; completion and per-routine output are
opt-in. Every exclusive cycle total and spill count reconciles with the
independent workload counter. All sixteen full-root listings also pass the
actual LF/CRLF census against their XEX images.

No compiler/IR contract changed, so no new shared-contract or full backend run
was needed. The pinned baseline retains Stage 7's complete qualification;
counterfactuals have only the audit checks described above.

From the repository root, with the qualified compiler retained:

```sh
python3 surveys/tn/audit-current-output.py --check
python3 surveys/tn/audit-current-output.py \
  --compiler target/tn-size-reduction/final/actionc-qualified \
  --out-dir target/tn-output-audit
for audit_variant in masks predicate combined; do
  python3 surveys/tn/audit-current-output.py \
    --compiler target/tn-size-reduction/final/actionc-qualified \
    --experiment "$audit_variant" \
    --out-dir "target/tn-output-audit/$audit_variant"
done
```

Run the affected test targets; the environment variables enable the audit and
select the alternate root or isolated source directory:

```sh
export CARGO_TARGET_DIR="$PWD/target/tn-output-audit/vm-target"
export CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_TEST_OPT_LEVEL=2
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test tn_codegen_reproducers -j2 -- --nocapture \
  > target/tn-output-audit/reproducers-vm.log
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test tn_size_profile -j2 -- --nocapture \
  > target/tn-output-audit/profile-default.log
for audit_program in TN TNDBG; do
  ACTIONC_TN_PROFILE_ROUTINES=1 ACTIONC_TN_PROFILE_SOURCE="$audit_program" \
    cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
    --test tn_size_profile -j2 -- --nocapture \
    > "target/tn-output-audit/$audit_program-profile.log"
done
for audit_variant in masks predicate combined; do
  ACTIONC_TN_PROFILE_ROUTINES=1 ACTIONC_TN_PROFILE_SOURCE=TN \
    ACTIONC_TN_PROFILE_DIRECTORY="$PWD/target/tn-output-audit/$audit_variant/source" \
    cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
    --test tn_size_profile -j2 -- --nocapture \
    > "target/tn-output-audit/$audit_variant-profile.log"
done
ACTIONC_TN_PROFILE_ROUTINES=1 ACTIONC_TN_PROFILE_SOURCE=TNDBG \
  ACTIONC_TN_PROFILE_DIRECTORY="$PWD/target/tn-output-audit/combined/source" \
  cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test tn_size_profile -j2 -- --nocapture \
  > target/tn-output-audit/combined-TNDBG-profile.log
for audit_case in escaping-record-address captured-call-arguments bounded-bit-mask; do
  target/tn-size-reduction/final/actionc-qualified --mode mir6502 --runtime cart \
    --listing "target/tn-output-audit/$audit_case.asm" \
    -o "target/tn-output-audit/$audit_case.xex" \
    "surveys/tn/reproducers/$audit_case.act"
done
python3 surveys/tn/output-audit-2026-10-12/collect.py --check
```

The collector requires the ignored audit listings, XEX images and profile
logs, plus the retained `target/debug/actionc-emit` used for IR inspection.
`collect.py` without `--check` writes updated measurements; review changed
inputs and explain them before replacing this pinned report. Neither script
modifies production sources. Rebuildable audit cache may be removed after
keeping the reports, logs and test executables.
