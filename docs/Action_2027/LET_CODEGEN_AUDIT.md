# LET code-generation audit

Date: 2026-09-07. Compiler baseline: `c29329e` on `main`.
Status: audit complete; no compiler or optimizer behavior changed.

## Conclusion

LET does not generally impose a MIR6502 execution penalty. Existing storage
forwarding and dead-store/home elimination remove the straight-line byte and
word bindings, and loop execution matches the reused-variable control.

There is a real bounded-relay coverage gap: separate single-definition homes
miss the existing two-definition relay selector. The parameter-indexed table
case costs ten extra bytes and ten extra CPU cycles with LET. However, blindly
promoting more homes is not justified: in a global-pointer variation, the
already-promoted mutable version selects a more expensive generic indexed-copy
path than the unpromoted LET version.

Every measured LET result matches the third control, which uses distinct
ordinary mutable locals. These opportunities concern general storage and
instruction selection, not LET-specific optimization or immutable metadata.

Recommended implementation order: repair static-table selection inside the
existing mixed indexed-copy path, then cost a bounded single-definition relay
extension using the current promotion machinery. Keep volatile-boundary storage
elimination separate until its alias/observability proof is explicit.

## Method and reproduction

The [audit harness](../../tools/vm-runtime-tests/tests/let_codegen_audit.rs)
generates nine small programs in three forms:

1. Reuse one mutable `value` local for successive definitions.
2. Give each definition a distinct ordinary mutable local (`fresh0`, etc.).
3. Use sequential immutable LET bindings.

All assignments and LET initializers execute at the same source points. The
fresh-local control changes names/storage identities without introducing LET
semantics. Loop bindings are inside an explicit BEGIN/END. The fresh control's
routine-level declarations have the same static storage lifetime on Atari.

The public compiler API builds each form in modern classic and MIR6502 with
cart and standalone runtimes: 108 builds. Each executes six input seeds and
both branch/index flags, for 1,296 checked executions. Independent host formulas
check output bytes/words, the call's side effect, both branch outcomes, the
16-iteration sum, and selected versus neighboring table elements.

```sh
cd tools/vm-runtime-tests
audit_dir=$(mktemp -d /tmp/actionc-let-audit.XXXXXX)
ACTIONC_LET_AUDIT_DIR="$audit_dir" cargo test --locked --test let_codegen_audit -- --nocapture
```

The optional directory must exist and be empty; artifacts are never overwritten.
Without the variable, the test owns and cleans a temporary directory. Retained
artifacts include all source forms, raw/optimized NIR, storage-analysis facts,
pre/post-materialization MIR, source listings, XEX files, aggregate measurements,
and the individual execution measurements. The MIR inspection artifacts use
the direct semantic/NIR pipeline and optimized cart configuration; executable
size and timing always come from the public compiler's linked output.

XEX size includes headers and emitted data, not only instructions. The origin
is $3000. Cycles are VM CPU instruction cycles from the initialized object entry
through the completion-marker store, excluding the terminal idle loop. Counts
include call/parameter setup and the common completion store. They are not PAL
frame timings, emulator wall time, DMA-aware timings, or AES benchmark results.
Table inputs are constrained to 0..15; word seeds include $7FFF and $FFFF.

The test asserts correctness, not exact sizes/cycles, so future improvements do
not require preserving today's inefficiencies. This is a bounded code-quality
audit, not exhaustive alias, MMIO-access-count, recursion or register-pressure
validation. Existing LET semantic/runtime regressions remain the broader
correctness coverage.

## Measurements

Each cell is **XEX bytes / CPU cycles**, mutable reused-home form followed by
LET. Cart and standalone are identical in these probes; they use no selected
SYS services. The distinct-mutable-local form matches LET in every reported
size, cycle range, and raw/optimized NIR local-access count. Full measurements
are preserved in [LET_CODEGEN_AUDIT.csv](LET_CODEGEN_AUDIT.csv).

| Probe | Classic: mutable → LET | MIR6502: mutable → LET |
| --- | --- | --- |
| One byte binding | 34 / 22 → 34 / 22 | 29 / 18 → 29 / 18 |
| Three byte definitions | 42 / 32 → 44 / 32 | 31 / 20 → 31 / 20 |
| Three CARD definitions | 77 / 76 → 78 / 72 | 65 / 68 → 65 / 68 |
| Global-pointer table relay | 83 / 69 → 84 / 69 | 85 / 76 → 83 / 69 |
| Parameter-indexed table relay | 98 / 95 → 99 / 95 | 76 / 65 → 86 / 75 |
| Snapshot across branch | 56 / 39–50 → 57 / 39–50 | 52 / 35–46 → 52 / 35–46 |
| Snapshot across call | 49 / 50 → 50 / 50 | 45 / 46 → 45 / 46 |
| Snapshot across volatile read | 48 / 40 → 49 / 40 | 44 / 36 → 48 / 40 |
| Sixteen-iteration byte chain | 67 / 809 → 68 / 809 | 60 / 713 → 60 / 713 |

## What fires, and what does not

### Straight-line calculations and loops

The pipeline in [nir.rs](../../src/nir.rs) runs value optimization, storage
propagation, home promotion, home elision, then final value optimization.
[Storage propagation](../../src/nir/storage_optimizer.rs) forwards the immediate
binding reads in the byte/CARD chains. [Home elision](../../src/nir/home_elision.rs)
then removes their stores and local storage. Both source forms reach zero local
homes/loads/stores for straight-line chains. In the loop only the induction
home remains. There is no need for a LET-specific SSA construction or optimizer.

Classic consumes SemIR through its existing projection, not optimized NIR.
Its register/value facts avoid many reloads, but the distinct declarations still
reserve separate storage. This accounts for small size increases with unchanged
byte execution costs. In the CARD chain, the distinct-home form also avoids one
absolute high-byte reload (four cycles), while allocating four additional data
bytes; net XEX growth is one byte. This is not evidence of a general LET speedup.

### Bounded relay selection

Both table probes contain the same value chain:

```action
LET value=target(index)
LET value=table(value)
target(index)=value
```

Storage forwarding does not simply erase these homes. Its existing
`retain_available_storage_values` pressure policy drops a stored temp when the
original use-def graph has no later direct use of that temp. Table-address
formation and the following pointer/index loads expose this boundary: the
future read is from the home, not from the original temp.

The [bounded-relay tier](../../src/nir/promotion.rs) repairs the reused-home
case: one byte home, two alternating store/load pairs, bounded gaps, one block,
no disqualifying barrier. It reuses the existing SSA renamer and removes that
home. Each distinct LET/fresh home has only one pair and does not meet the
two-pair profitability threshold. Optimized NIR therefore retains two homes,
two loads and two stores in those forms.

For the parameter-indexed probe, existing MIR pointer/index and accumulator
selection then composes the promoted chain as:

```asm
LDA (ptr),Y
TAY
LDA table,Y
LDY index
STA (ptr),Y
```

The unpromoted form instead stores the first binding, loads it into Y, and
stores the second binding before the final pointer write. Replacing those two
stores and one load with TAY explains the ten-cycle gap. Eight instruction bytes
plus two local bytes explain the ten-byte XEX gap. The JSR to Map remains in
both listings; leaf inlining is not responsible for the difference.

### Why broader promotion needs an instruction-selection fix first

The global-pointer probe reloads its pointer cell for the final store. Its
promoted NIR has no relay homes, yet materialized MIR builds two indirect
pointers and spills the table index. The table read becomes an indirect read
after full address arithmetic, instead of `LDA table,Y`.

This matches the generic `indexed-byte-copy` path in
[materialize/indexes.rs](../../src/mir6502/materialize/indexes.rs): after the
same-base attempt, it builds both source and destination pointer addresses.
The unpromoted form keeps the table load separate, so existing static-table
indexing selects absolute indexed-Y. Its local traffic is cheaper than the
generic copy's address setup: LET is two bytes smaller and seven cycles faster.

There is already static-byte-index canonicalization and direct indexed-copy
selection. Extend that machinery to a static source and pointer destination,
preserving the captured index, pointer dependencies, Y restoration, and memory
access order. Do not add a source-pattern matcher. A mere threshold reduction
in NIR promotion could erase the parameter-indexed gap while making this
global-pointer case worse.

### Branches, calls and volatile accesses

Branch and call probes retain one genuinely used snapshot home in optimized
NIR. Earlier obsolete definitions disappear. Both forms have the same MIR size
and execution cost. Cross-block forwarding deliberately refuses to introduce
a new temp live range just to replace a source-home reload; this is the existing
pressure safeguard, not failed name resolution or incorrect LET lowering.

Volatile reads clear cached storage values. In addition, home elision's backward
transfer marks **every candidate home live** at a volatile operation. The older
LET home has no remaining source reads but its initial store survives this
barrier. A reused mutable home overwrites its old value, so its earlier store
can disappear. MIR consequently emits one extra absolute STA and one data byte
for LET: four extra cycles and four extra XEX bytes.

Immutability alone is not sufficient permission to remove that barrier. Atari
locals have routine-static backing, not the proven-private automatic activation
used on native targets. Any relaxation must distinguish effect ordering from
possible observation/aliasing using reusable storage/effect facts. It must not
move, merge, duplicate or discard the volatile operation itself.

## Proposed bounded follow-up slices

1. Extend existing MIR indexed-copy selection to retain efficient static-byte
   table reads with pointer destinations. Validate both table probes, index and
   pointer alias/capture hazards, page edges and baseline/optimized execution.
2. Extend the existing NIR bounded-relay profitability policy to connected
   short-lived single-definition homes, or an equivalently bounded forwarding
   rule. Reuse storage eligibility, liveness/dominance and `promote_home`; retain
   cold-home and call/volatile pressure guards. Measure both table shapes and
   the original AES relay, not just the newly favorable case.
3. Audit volatile-boundary dead stores separately. Generalize existing effect
   and alias proofs before weakening the all-homes-live rule. The measured
   four-cycle opportunity does not justify assuming routine-static privacy.

No implementation of these optimization slices is included in this audit.

## Validation

- Audit: 108 builds and 1,296 independently checked executions pass.
- Full compiler suite: 2,845 tests pass.
- NIR snapshots pass unchanged; the NIR sweep passes all 38 fixtures.
- Full locked VM suite: 137 tests pass, including the new audit test.
- Compiler sources and existing fixtures are unchanged; no performance
  expectations are frozen into the test suite.
