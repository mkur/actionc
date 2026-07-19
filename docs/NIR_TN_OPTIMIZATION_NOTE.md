# TN NIR Optimization Survey And Work Note

Snapshot date: 2026-07-19.

This note records a census of the verifier-clean NIR generated for the modern
profile and MIR6502 backend from `samples/tn/modern/TN.ACT`. Its purpose is to
guide the next NIR optimization work without moving 6502-specific strategy into
NIR.

The central finding is that TN does not need more temporary-value propagation
first. Almost every NIR temporary has one use, and no temporary crosses a basic
block boundary. Source-level values cross blocks through local, parameter, and
global storage instead. The next high-leverage work is therefore conservative
storage-to-value promotion, followed by an explicit representation for values
that merge at CFG joins.

## Reproducing The Listing

Generate the surveyed listing with:

```sh
mkdir -p target/analysis
cargo run --quiet --bin actionc-emit -- \
  --profile modern \
  --backend mir6502 \
  --emit-nir \
  samples/tn/modern/TN.ACT \
  > target/analysis/TN-modern-unoptimized.nir
```

The surveyed output contained 3,412 lines and 80,351 bytes. Its SHA-256 was:

```text
0ed9ad15f04b64dd395f264541ed9445912ac4a9651573d9e606be280c7ccf1b
```

`--emit-nir` currently prints verified NIR before `nir::optimize_program` runs.
Use `--emit-optimized-nir` to print the exact post-optimizer NIR passed to
MIR6502, and `--emit-nir-stats` to compare deterministic lowered and optimized
censuses. The storage conclusions in this note remain valid because the current
NIR optimizer propagates temporary values and offsets but does not maintain
facts for local, parameter, or global storage.

Future surveys can therefore compare pre-optimization NIR, optimized NIR,
MIR6502, and final bytes without adding temporary instrumentation.

## Whole-Program Census

| Item | Count |
| --- | ---: |
| Routines | 105 |
| Basic blocks | 487 |
| Temporary definitions | 1,324 |
| Loads | 762 |
| Stores | 351 |
| Calls | 342 |
| Machine blocks | 48 |
| Conditional branches | 154 |
| Gotos | 213 |
| CFG joins with more than one predecessor | 122 |

The 1,113 loads and stores split as follows:

| Storage shape | Accesses |
| --- | ---: |
| Globals | 524 |
| Locals | 243 |
| Parameters | 149 |
| Indexed places | 108 |
| Dereferenced pointers | 45 |
| Absolute memory | 44 |

Direct local and parameter storage accounts for 392 accesses. Direct globals
account for another 524 accesses, but globals require stronger call, alias, and
machine-effect facts before they can be propagated safely.

## Temporary-Value Shape

Of the 1,324 temporary definitions:

- 1,312 have exactly one use;
- five have no use, all of them call results;
- seven have more than one use;
- none are used from another basic block.

This explains why routine-wide temporary constant and alias propagation has not
changed TN: there is no cross-block temporary flow to optimize. NIR lowering
normalizes expressions into useful typed temporaries, but every block reloads
the source variable homes it needs.

All 154 conditional branches consume a compare temporary defined in the same
block. There are no literal branch conditions and no constant-versus-constant
branch comparisons. Sparse executable-edge propagation therefore has no TN
edge to prune at this stage.

There are 189 compares in total. Of those, 153 compare against a constant and
62 compare against zero. Those shapes may become useful for edge constraints
and range propagation after storage values can survive across blocks.

## Hot Routines

| Routine | Memory operations | Local/parameter | Global | Calls | Blocks |
| --- | ---: | ---: | ---: | ---: | ---: |
| `SetWin` | 152 | 30 | 94 | 16 | 36 |
| `Handle` | 104 | 33 | 60 | 31 | 67 |
| `Window` | 85 | 10 | 59 | 9 | 10 |
| `Copy` | 84 | 58 | 15 | 37 | 23 |
| `Sort` | 41 | 12 | 23 | 1 | 13 |

`SetWin`, `Handle`, `Window`, and `Copy` alone account for 425 memory
operations, approximately 38% of TN's total. They also contain 136 blocks,
approximately 28% of the whole-program CFG.

Prominent ordinary scalar home candidates include:

| Home | Direct accesses |
| --- | ---: |
| `Handle::ch` | 25 |
| `Copy::j` | 15 |
| `InputLine::curpos` | 12 |
| `PopUp::menu` | 11 |
| `Sort::gap` | 10 |
| `Fnamecmp::s` | 10 |
| `Fnamecmp::t` | 10 |
| `Copy::mem` | 8 |
| `Copy::len` | 7 |
| `Copy::k` | 7 |
| `Copy::files` | 7 |
| `Copy::flag` | 6 |

A conservative syntactic census finds 43 ordinary, non-address-taken scalar
locals and 94 non-address-taken parameters, together responsible for 359 direct
accesses. This is an eligibility ceiling, not a count of immediately removable
homes. Definite initialization, Action! persistent storage semantics, aliases,
machine visibility, and omitted-parameter behavior must still be respected.

## Straight-Line Storage Opportunities

An exact-location, block-local survey that treats calls, machine blocks,
indexed stores, indirect stores, and absolute stores as barriers finds 86 loads
whose value is already available in the same block:

- 43 follow an earlier load from the same place;
- 43 follow an earlier store to the same place;
- 25 access locals;
- 14 access parameters;
- 47 access globals.

The largest concentrations are:

| Routine | Candidate loads |
| --- | ---: |
| `SetWin` | 15 |
| `Handle` | 11 |
| `Copy` | 7 |
| `Window` | 6 |
| `Sort` | 6 |

For example, a `SetWin` block loads `pathBuf`, stores or derives a new value,
and then reloads `pathBuf` while that exact value is still known. This can be
rewritten without any CFG merge representation.

TN also contains 91 adjacent load/modify/store shapes: 75 additions, 14
subtractions, and two XOR operations. These are mostly counters and pointer
updates. They are evidence for scalar promotion, not a reason to add a
target-specific increment operation to NIR.

## Structured Memory-Effect Precision

SemIR records read and write regions. Phase 3 now preserves them in NIR as:

```text
None
Regions([{ stable identity, offset, size }, ...])
Unknown
All
```

The storage optimizer invalidates only tracked homes whose byte range overlaps
a structured call write. Unresolved, unknown, opaque, indirect, OS, and
recursive effects remain conservative full barriers. Unannotated direct calls
continue to clear globals because complete interprocedural summaries are not
yet inferred for the source pipeline.

This matters for TN because it contains 342 calls. It is particularly visible
in `Copy`, which has 58 local accesses and 37 calls. Treating every call as a
full barrier would restrict promotion to short fragments. The exact-region
representation now supplies the identity needed by later scalar promotion and
synchronization work when structured summaries are available.

Machine blocks should remain opaque full barriers unless their structured
effects prove something narrower. Absolute memory and hardware-visible storage
must remain conservative.

## Ranked Work

### 1. Add Observable Optimized-NIR Output

Add either `--emit-optimized-nir` or a stage selector such as:

```text
--emit-nir-stage lowered
--emit-nir-stage optimized
```

Acceptance criteria:

- lowered output remains useful for migration and verifier inspection;
- optimized output is exactly the NIR consumed by MIR6502;
- the output is deterministic and suitable for fixture or census tooling;
- no optimization behavior changes in this slice.

### 2. Implement Conservative Block-Local Storage Forwarding

Introduce exact storage facts for ordinary locals and parameters first:

```text
StorageIdentity -> NirValue
```

Update a fact after a direct store and reuse it for a later direct load. Kill a
fact when the same place is overwritten. Initially clear relevant facts at
calls, machine blocks, and aliasing writes.

Eligibility for the first slice should exclude:

- address-taken storage;
- absolute storage;
- local or global aliases;
- indexed and dereferenced places;
- storage with unresolved initialization or persistence behavior.

Acceptance criteria:

- focused load/store/load and store/load fixtures lose the redundant load;
- joins are not crossed in this slice;
- calls and opaque machine blocks are barriers;
- verifier-clean NIR is preserved;
- `SetWin`, `Copy`, and `Sort` show measurable NIR reductions.

### 3. Preserve Structured Effect Regions

Carry optimizer-grade region identities from SemIR into NIR rather than only a
region count. Relate direct local, parameter, global, absolute, indexed, and
unknown memory effects to stable storage identities where possible.

Acceptance criteria:

- a call known not to write a tracked home preserves its fact;
- a call that may write the home kills the fact;
- unknown or opaque effects kill all possibly observable memory facts;
- MIR6502 consumes NIR effects and never consults SemIR to recover them.

### 4. Extend Storage Facts Across The Routine CFG

Reuse the routine-level forward data-flow framework. At a join, retain a
storage fact only when every executable predecessor supplies the same value.
Sparse executable-edge information may exclude infeasible predecessors.

This slice still requires no phi or block-parameter representation.

Acceptance criteria:

- dominance-safe facts cross single-predecessor blocks and agreeing joins;
- loop entry does not incorrectly reuse a value changed by the back edge;
- calls and writes apply precise fact kills;
- existing sparse-edge and verifier tests remain green.

### 5. Add Typed Block Parameters Or Phi Nodes

TN has 122 CFG joins and no cross-block temporary uses. Full scalar promotion
needs a typed representation for different incoming values that merge at a
block.

The representation must be target independent. NIR should represent the value
merge; MIR6502 should decide registers, copies, spill homes, and ABI placement.

`Sort` is the preferred first acceptance routine because it has a conventional
loop, ten accesses to `gap`, and only one call. Later acceptance routines should
include `Copy::j` and `Handle::ch`.

Acceptance criteria:

- verifier checks predecessor arguments, arity, types, and dominance;
- printer output remains readable;
- loop-carried scalar values can remain NIR values instead of storage traffic;
- MIR6502 lowers parallel edge copies safely;
- no 6502 register or flag concepts enter NIR.

### 6. Eliminate Dead Stores And Unneeded Homes

After promotion, remove stores that cannot be observed before another store or
routine exit. Remove a local or parameter backing home only when all semantic
uses are represented by promoted values and no address, alias, machine block,
ABI rule, or persistence rule requires storage.

MIR6502 may still create a transient spill location if target register pressure
requires one. That spill is a target allocation decision, not a reason to keep
the original source-variable home in NIR.

## Small Follow-Up Opportunities

Nine same-block duplicate `AddrOf` operations are candidates for exact common
subexpression elimination. This is a safe, low-value cleanup once place identity
and width checks are in place.

Five call-result temporaries are unused. NIR should eventually represent an
explicitly discarded result, but materialized MIR6502 already drops these
result homes. This is primarily an IR cleanliness improvement, not an expected
TN byte reduction.

Branch range facts, compare canonicalization, and condition reasoning should be
revisited after storage promotion exposes cross-block values. They have little
TN leverage while every block reloads its operands.

## Target Boundary

The following remain MIR6502 responsibilities:

- A/X/Y and status-flag use;
- compare-to-branch flag fusion;
- scaled indexed addressing selection;
- zero-page pools and physical home placement;
- spill decisions;
- helper selection and 6502 peepholes.

NIR should supply typed values, storage identity, CFG merges, and conservative
effects. That structure will also be reusable by a future Z80 MIR without
encoding 6502 strategy in NIR.

## Recommended Implementation Order

1. Add optimized-NIR output and repeatable census tooling.
2. Implement conservative block-local local/parameter forwarding.
3. Preserve structured memory-region identities through NIR.
4. Propagate agreeing storage facts across the routine CFG.
5. Add typed block parameters or phi nodes and promote loop-carried scalars.
6. Remove dead stores and source homes made unnecessary by promotion.

Each slice should report four measurements for TN: optimized NIR operations,
MIR6502 logical homes, final static bytes, and relevant runtime validation. NIR
operation counts are evidence of structural improvement, but final byte savings
must be measured rather than inferred from the number of removed loads.
