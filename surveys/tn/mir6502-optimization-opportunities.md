# TN MIR6502 Listing Optimization Survey

Status: forward-branch relaxation implemented; remaining backlog ranked

Date: 2026-07-18

Baseline commit: `19b11ae5345eabd8113be9e004b269719625dca2`
(`Elide internal parameter storage`)

Scope: `samples/tn/modern/TN.ACT`, `--profile modern --backend mir6502`

## Reproducing the artifacts

The measurements in this note came from fresh artifacts under
`target/tn-mir-analysis/`. Run these commands on the current tree for the
post-relaxation result; check out the baseline commit above to reproduce the
pre-relaxation columns:

```sh
mkdir -p target/tn-mir-analysis

ACTIONC_MIR6502_PEEPHOLES=per-routine \
  cargo run --quiet --bin actionc-emit -- \
    --profile modern --backend mir6502 --emit-listing \
    samples/tn/modern/TN.ACT \
    > target/tn-mir-analysis/TN-mir6502.lst \
    2> target/tn-mir-analysis/TN-mir6502.peepholes

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-materialized-mir6502 \
  samples/tn/modern/TN.ACT \
  > target/tn-mir-analysis/TN-mir6502-materialized.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-map \
  samples/tn/modern/TN.ACT \
  > target/tn-mir-analysis/TN-mir6502.map

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/tn/modern/TN.ACT \
  > target/tn-mir-analysis/TN-mir6502.xex

cargo run --quiet --bin actionc-listing-quality -- \
  target/tn-mir-analysis/TN-mir6502.lst \
  > target/tn-mir-analysis/TN-mir6502-quality.txt
```

The same listing, map, load, and quality commands were run with
`--backend classic` as a directional comparison. Classic is not a byte-level
oracle for MIR6502: it interleaves some routine storage with code and makes
different target choices. It is still useful for locating disproportionate
MIR materialization traffic.

## Pre-relaxation baseline

| Metric | MIR6502 | Modern/classic | Difference |
| --- | ---: | ---: | ---: |
| Load file | 14,145 bytes | 10,445 bytes | +3,700 (+35.4%) |
| Primary load segment | 14,133 bytes | 10,433 bytes | +3,700 |
| Listing instructions | 5,752 | 4,338 | +1,414 |
| `LDA` + `STA` instructions | 3,089 | 1,910 | +1,179 |
| `LDA` + `STA` share | 53.7% | 44.0% | +9.7 points |
| `JMP` instructions | 333 | 158 | +175 |
| Spill cells | 71 bytes | 0 | +71 bytes |

The listing-quality parser interprets bytes inside a few machine/data PROC
ranges as instructions. Consequently its instruction and code-byte totals are
best used comparatively, not as authoritative segment accounting. The load
file and address-range measurements are authoritative.

MIR6502 places 1,330 bytes before `<program>` at `$3132`:

| Central region | Range | Bytes |
| --- | --- | ---: |
| Globals | `$2C00-$2C77` | 120 |
| Static strings/data | `$2C78-$2E52` | 475 |
| Routine parameter/local/spill storage | `$2E53-$3131` | 735 |

The 735 routine-storage bytes comprise 134 parameter bytes, 71 one-byte spill
cells, and 530 other routine-local bytes. Most of the latter are real
addressable Action! storage and are not an optimization target by themselves.

Across common named routine ranges, MIR6502 is 2,438 bytes larger than classic.
Its separate `<program>` range adds another 49 bytes. The remaining 1,213 bytes
of the total gap are in central storage/layout rather than those matched
ranges. This is a layout decomposition, not a claim that 1,213 bytes are dead:
classic includes some routine storage inside its PROC ranges while MIR6502
centralizes it.

The five largest routine deltas account for 1,013 bytes, about 41% of the
matched-routine plus `<program>` gap:

| Routine | MIR6502 bytes | Classic bytes | Delta | MIR spill accesses |
| --- | ---: | ---: | ---: | ---: |
| `Handle` | 1,163 | 839 | +324 | 29 |
| `Copy` | 947 | 723 | +224 | 22 |
| `SetWin` | 1,403 | 1,233 | +170 | 70 |
| `SwapScr` | 352 | 196 | +156 | 24 |
| `Xloop` | 360 | 221 | +139 | 26 |

`SetWin` remains the largest absolute routine and the strongest individual
materialization hotspot. `Handle` has the largest size delta.

## Forward-branch relaxation result

Measured iterative branch relaxation reduces the TN MIR6502 load file from
14,145 to 13,773 bytes, a 372-byte (2.6%) saving. The primary load segment
shrinks by the same amount, from 14,133 to 13,761 bytes.

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Load file | 14,145 bytes | 13,773 bytes | -372 |
| Primary load segment | 14,133 bytes | 13,761 bytes | -372 |
| Listing instructions | 5,752 | 5,626 | -126 |
| `JMP` instructions | 333 | 207 | -126 |
| Branch-over-`JMP` shapes | 164 | 42 | -122 |
| Branch-over-`JMP` shapes whose final target fits | 117 | 0 | -117 |

The result exceeds the initial 351-byte estimate for three reasons. Relaxing
the 117 initially reachable targets shortened the layout enough for four more
targets to enter relative range, adding 12 bytes. One target at an initial
offset of +129 became reachable when accounting for the three bytes removed by
its own relaxation. Two unconditional jumps to the immediately following block
also disappeared, adding six bytes. Thus the total is
`(122 * 3) + (2 * 3) = 372` bytes. The 42 remaining veneers target blocks
outside relative range in the converged layout.

## Ranked optimization opportunities

The ranking combines expected static-byte impact, confidence in the listing
evidence, architectural fit, and implementation risk. It is also the
recommended implementation order.

| Rank | Opportunity | Evidence / ceiling | Expected impact | Risk |
| ---: | --- | --- | --- | --- |
| 1 | Forward conditional-branch relaxation (implemented) | 122 branch-over-`JMP` pairs relaxed after convergence; two fall-through jumps removed | Measured 372 bytes | Complete |
| 2 | Reduce spill pressure through value propagation, scheduling, and transient-home elimination | 71 cells and 276 absolute spill accesses consume 899 data-plus-code bytes; 720 copy-propagation candidates, 964 replaceable temp uses, and 633 retained unhandled reads remain | Hundreds of bytes; largest remaining ceiling, although the 899-byte gross ceiling is unattainable | Medium-high, best delivered in narrow consumer slices |
| 3 | Elide unused and lazy direct-ABI parameter homes | 134 parameter bytes: 56 entirely unreferenced, 36 direct-ABI, 42 SARGS; direct capture costs 108 bytes | Immediate 56 data bytes, then up to 144 bytes plus reload traffic from direct homes | Medium; address escape and call/CFG lifetime must remain explicit |
| 4 | Scaled `(zp),Y` addressing for two-byte indexed elements | 34 genuine `ASL/PHP/CLC/ADC/.../PLP` full-address sites | About 100-136 bytes; nominal gross ceiling is 136 | Medium; carry and two-address consumers need proof-guided lowering |
| 5 | Fold byte add-one read/modify/write sequences to `INC` | Six exact `LDA m; CLC; ADC #1; STA m` sites | 32 bytes when carry/overflow are dead | Low-medium; requires flag-liveness check |
| 6 | Tail-call `JSR`/`RTS` pairs | Four exact sites | 4 bytes | Low |

### 1. Relax forward conditional branches after layout (implemented)

`emit_terminator` can use a relative branch when a target is the next block or
when a previously bound, usually backward, label is in range. An unbound
forward target falls back to:

```text
B!cond skip
JMP then
skip:
JMP else
```

The baseline listing showed that 117 of these `JMP then` targets were already
within the 6502 relative range. Replacing each leading branch-plus-jump pair
with the direct conditional branch established the original 351-byte floor.

The emitter now performs measured, monotonic relaxation as part of deferred
layout convergence. It records actual block and branch positions, admits only
targets whose signed displacement fits, re-emits after every newly admitted
set, and leaves the final relative-patch range check authoritative. A bounded
trial emission handles a target that becomes reachable only after replacing
its own three-byte veneer. The emitter also prefers the non-fall-through edge
as the conditional target, inverting the branch when necessary, and omits a
jump when the other successor is the next block. Multi-flag conditions validate
every branch that shares a relaxed target. Far targets keep the safe
branch-over-absolute-`JMP` veneer.

On TN the iterative shortening admitted four targets beyond the original 117,
and self-relaxation admitted one more, so 122 veneers were removed. Together
with the two fall-through jumps, this produced the measured 372-byte result
above.

### 2. Reduce spill pressure and eliminate transient homes

This has the largest strategic ceiling. The current passes are useful but have
not exhausted the MIR facts they already discover:

- 56 redundant reloads, 39 dead temp definitions, seven dead direct-load lane
  definitions, 34 constant uses, and 34 direct-memory uses are already removed;
- only 13 consumer forwards and three cross-block forwards currently fire;
- the v2 census still reports 720 copy-propagation candidates, 964 replaceable
  temp uses, 23 memory-forward candidates, and 633 stores retained because the
  read consumer is not handled;
- 95 single-edge seeds yield only three cross-block forwards, while 143 joins
  and 44 backedges are conservatively skipped.

The upper-bound counters do not mean that every reported use can be replaced.
They identify where the next vertical slices should work. The hottest candidate
counts are:

| Routine | Copy candidates | Replaceable temp uses |
| --- | ---: | ---: |
| `SetWin` | 76 | 93 |
| `Copy` | 56 | 68 |
| `SwapScr` | 40 | 53 |
| `Window` | 40 | 48 |
| `Handle` | 38 | 44 |
| `Rename` | 34 | 48 |

Recommended slices:

1. Extend consumer rewriting for the 633 currently classified unhandled reads,
   starting with direct byte/word stores, call arguments, compare operands, and
   indexed-address inputs.
2. Remove the now-dead producer and allocate no spill/home for a temp that no
   longer reaches an addressable use.
3. Extend across single-predecessor, non-backedge block boundaries. Do not merge
   facts through joins until dominance and lane identity prove the value.
4. Preserve memory facts across calls only when structured, transitive effects
   prove that the particular storage cannot be read or written. Calls currently
   kill 1,036 v2 facts and 40 reloads are explicitly retained at call barriers;
   weakening this conservatism globally would be unsafe.

The remaining 29 adjacent `STA m; LDA m` listing pairs should not become a raw
emission peephole. Many sit at loop headers or join labels with alternate
predecessors. They are useful regression metrics for CFG-aware propagation.

Spill traffic is the concrete first target of this work. The 71 surviving
one-byte spill cells receive 147 reads and 129 writes in the final listing:
276 absolute-memory instructions occupying 828 code bytes. Including the 71
storage bytes gives a 899-byte gross footprint. It is not a realizable saving
because values live across calls, joins, or constrained-register consumers
need a stable home, but it shows that spill pressure is a substantial part of
the MIR/classic gap rather than a 71-byte data-layout detail.

| Routine | Spill reads | Spill writes | Total traffic |
| --- | ---: | ---: | ---: |
| `SetWin` | 37 | 33 | 70 |
| `Handle` | 14 | 15 | 29 |
| `Xloop` | 14 | 12 | 26 |
| `SwapScr` | 16 | 8 | 24 |
| `Window` | 14 | 9 | 23 |
| `Copy` | 12 | 10 | 22 |
| `Rename` | 10 | 10 | 20 |

These seven routines account for 214 of 276 accesses, or 77.5%. They should be
the acceptance corpus for spill-pressure work.

The backend is not starting from zero. After temp materialization it already
removes dead spill stores, folds several immediate consumers, colors
non-overlapping spills confined to one basic block, prunes unused slots, and
maps eligible block-local, non-call-crossing byte spills into free zero-page
locations in `$E0-$EF`. More slot coloring alone mainly reduces the 71 data
bytes; it does not remove the 828 bytes of access traffic.

The underlying limitation is that `materialize_temp_ops` lowers every surviving
virtual-temp definition through `A` and immediately stores it to a spill. The
best sequence of improvements is therefore:

1. Prevent the spill: forward or sink the producer into its consumer before
   temp materialization, including the unhandled store, compare, call-argument,
   and indexed-address consumers.
2. Add block-local constrained register scheduling for remaining short-lived
   values, keeping them in `A`, `X`, or `Y` until their last use when operand
   constraints and flags allow it.
3. Rematerialize cheap constants, addresses, and direct loads when repeating
   them costs less than a spill store plus reload.
4. Only then extend zero-page allocation or CFG-wide spill coloring. Zero-page
   values that cross calls require call-graph/clobber interference because
   routine-local `$E0-$EF` allocations currently overlap; global slot coloring
   requires normal CFG liveness rather than the current single-block interval.

A practical initial target is a measured 100-300 byte reduction concentrated
in the seven routines above. The exact result must come from each consumer
slice; treating all 899 bytes as removable would repeat the mistake of quoting
a theoretical spill ceiling as an estimate.

### 3. Allocate parameter homes only when required

All 134 signature parameter bytes currently receive central storage. They
separate cleanly into:

- 56 bytes in 18 machine-backed routines whose materialized MIR never refers
  to `MirMem::Param`; their custom machine blocks consume the ABI registers
  directly and the generated ABI prologue is already skipped;
- 36 bytes captured from the direct Action! ABI, producing 36 absolute stores
  or 108 code bytes at routine entry;
- 42 bytes used by 11 SARGS routines, where addressable contiguous storage is
  part of the ABI and should remain the conservative case.

The first slice is to omit the 56 proven-unreferenced storage bytes. The layout
must include structured machine-symbol references in that proof rather than
assuming every `MachineBlock` is independent of parameter labels.

The second slice is lazy homes for the 36 direct-ABI bytes. Model each incoming
byte as an ABI value, propagate it to uses, and allocate/capture a memory home
only when it is assigned, its address escapes, it crosses a clobber or CFG
boundary that needs persistence, or a machine consumer requires a symbol. The
144-byte data-plus-entry-store figure is a gross ceiling; some TN parameters
legitimately need homes. SARGS storage should not be generalized away in this
phase.

### 4. Use scaled `(zp),Y` for two-byte indexed elements

There are 34 real full-address sequences in MIR-generated code; one additional
`PHP` reported by the mnemonic counter is data inside the `drives` table. The
real sites are concentrated as follows:

| Routine | Sites |
| --- | ---: |
| `Sort` | 6 |
| `Copy`, `Handle`, `SetWin` | 4 each |
| `Xloop` | 3 |
| `Draw` | 2 |
| Eleven other routines | 1 each |
| **Total** | **34** |

These are emitted by the generic full-address path for
`MaterializeIndexedAddress`: scale the byte index, preserve its carry with
`PHP`, add both base bytes into a zero-page pointer, restore the scale carry,
and then consume the pointer with fixed `Y` offsets.

The modern/classic implementation has already validated the better target
strategy: keep `low(2*index)` in `Y`, propagate the `ASL` carry into the base
high byte, and use `(zp),Y`. Its TN work measured four gross bytes per migrated
site and 96 net bytes over 25 profitable migrations. Applying the same
proof-guided consumer classification to MIR gives a 136-byte nominal gross
ceiling and a realistic expectation near 100-136 bytes.

This should be represented as a MIR6502 addressing/consumer choice. Do not scan
emitted `ASL/PHP/.../PLP` bytes. Reuse the classic proof boundary for byte-ranged
indexes, carry-correct high-byte adjustment, Y lifetime, source staging, and
the zero-net-gain two-address cases.

### 5. Select `INC` when its flag contract is sufficient

The listing contains four zero-page and two absolute instances of:

```text
LDA m
CLC
ADC #1
STA m
```

`INC m` saves five bytes for a zero-page site and six for an absolute site, 32
bytes total. It preserves the result's N/Z flags but not the ADC C/V result, so
the MIR rewrite is legal only when carry and overflow are dead. Prefer an
operation-selection or late-MIR combine with explicit flag liveness.

### 6. Convert final calls to tail jumps

Four routines end in `JSR target; RTS`: `Delete -> Xloop`, `Attrib -> Xloop`,
`InitPanels -> Path`, and `NavError -> Handle`. `JMP target` saves one byte per
site and one stack round trip. This is safe only when the Action ABI, return
value placement, and machine-stack behavior match. The impact is too small to
precede the structural work above.

## Already-clean or low-value areas

- No routine begins with a `JMP` to its immediately following instruction.
  Routine-entry fall-through trampolines are not a MIR6502 problem at this
  baseline.
- There are no absolute `$00xx` accesses that should have used a zero-page
  opcode.
- There are no `LDA x; CMP #0; BEQ/BNE` sequences left.
- Spill cells consume only 71 static bytes, but their 276 accesses occupy 828
  code bytes. Eliminating unnecessary homes ranks highly; coloring those cells
  for storage alone does not.
- Only four address-reuse candidates are reported. A standalone address-cache
  project ranks below scaled addressing and general value propagation.
- The listing tool's data-like PROC reports are not optimization evidence;
  several are structured machine tables decoded as 6502 instructions.

## Validation gates for future work

For every implemented slice:

1. regenerate the MIR, materialized MIR, listing, map, load file, peephole
   report, and listing-quality report from the commands above;
2. record total load bytes and the affected routine ranges, not just optimizer
   counters;
3. add focused MIR6502 tests for the target shape and its rejected boundary;
4. run `cargo test` and the relevant MIR6502 stress sweep;
5. run TN under the isolated XL/no-cart configuration for runtime-sensitive
   addressing, ABI, and control-flow changes.

Value propagation must keep calls, absolute/hardware memory, pointer writes,
and machine blocks conservative unless structured facts prove otherwise. The
scaled-address and parameter-home work belong in MIR6502 target strategy and
layout; neither should require MIR6502 to recover semantic facts from SemIR.
