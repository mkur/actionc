# MIR6502 pointer byte constant arithmetic implementation

Prepared 2026-11-11. Status: slice 1 implemented; selection remains inert. Starting revision `553f8074`, with compiler
code at `6311c110`. Implement in four vertical slices, committing each after
its checks pass. This plan includes the proposed MIR contract; no separate
design note is needed for the bounded form below.

Select a shorter implementation of `p + BYTE[p+offset] + 1`, preserving the
ordered indirect read, complete 16-bit result and observable memory behavior.
The [Strcat audit](benchmarks/6502-argument-staging/STRCAT_BYTE_BREAKDOWN.md)
identifies a 26-byte implementation. A candidate using X occupies 21 bytes.
Five bytes is a local opportunity, subject to machine-state, alias and final
image qualification.

## Scope and ownership

The first implementation handles addition followed by constant `+1`, in one
block, over two adjacent already-materialized operations:

```text
OffsetPointerByIndirectByte(Add, dst=R, source=P, offset=k)
AddByteToWordMem(mem=R, value=ConstU8(1))
```

Admit complete, non-wrapping, physically disjoint fixed-zero-page pairs P and
R in ordinary RAM. Require `0 <= k <= 255` and a consumer that establishes Y
with that immediate offset. Keep scaled-Y, subtraction, other constants,
overlapping pairs and nonadjacent forms on their existing paths. This is the
first member of the general pointer/byte/constant family; broader constants
need separately qualified schedules.

SemIR and verified NIR retain their existing computation and source semantics.
MIR6502 owns fusion, register allocation within the sequence, flags and alias
proofs. Emission writes an explicitly selected operation and its local branch;
it must not recognize adjacent source expressions or decide whether fusion is
legal. No SemIR/NIR form or public ABI change is planned.

Keep argument capture stores, parameter homes, calls and post-call pointer
reconstruction unchanged. The audit's parameter-alias counterexamples remain
required controls. Sharing earlier parameter loads and retaining pointers
across a call are separate optimizations.

## Selected operation and candidate sequence

Use a narrowly typed MIR6502 operation, provisionally
`OffsetPointerByIndirectBytePlusOne { dst, source, offset }`. Its value contract
is `(pointer_value + zero_extend(loaded_byte) + 1) mod 65536`. The existing
base operation is still the fallback. A target operation keeps the conditional
high-byte repair explicit to MIR consumers without constructing new CFG blocks
for an instruction-local branch.

The operation reads the same source-pair bytes and one indirect byte as the
original sequence. It fully defines the destination word and does not consume
its incoming value. Its internal high-byte repair may read the high byte it
has just defined. Model the indirect read conservatively, including possible
observation of fixed homes. Declare A/X and C/N/Z/V clobbers, Y established to
`offset`, both destination writes, and no stack or call effect. Preserve the
backend's existing binary-arithmetic invariant with decimal mode clear.

For fixed zero-page pairs, investigate this 21-byte body:

```asm
        LDY #offset
        LDA P
        CLC
        ADC (P),Y
        TAX
        LDA P+1
        ADC #0
        INX
        STX R
        STA R+1
        BNE done
        INC R+1
done:
```

`TAX` and `LDA` preserve the carry from the low-byte addition. `INX` establishes
the incremented low byte and its zero flag; the two stores preserve that flag
for the high-byte repair. The result must handle low-byte carry, increment
wrap, high-byte overflow and full modulo-65536 wrap.

The final A and X differ from the baseline, as can all four arithmetic flags.
Require A/X and C/N/Z/V dead at the window exit, including successor, edge and
implicit machine/call inputs. Y has the same final value. Initial selection
must reject live state rather than adding speculative repair instructions.

## Required memory and rewrite proofs

The candidate removes intermediate destination accesses and changes the order
of its final word stores. Establish all of the following through shared facts:

- The source pair has complete incoming definitions. The destination is a
  complete two-byte output range; it needs no incoming definition for the
  arithmetic itself. Their final physical ranges are distinct and neither
  pair wraps through `$FF` to `$00`.
- Preserve the original prefix: establish Y, read source-low, then perform the
  one indirect byte read. Read source-high afterward, as before. No source read
  moves across another executable operation.
- The removed intermediate destination write cannot affect the subsequent
  source-high read, because the physical pairs are disjoint. All removed
  destination reads are internal consumers of definitions replaced by the
  fusion. Preserve the final word value for every surviving reader.
- The indirect target may alias the destination's incoming contents or fixed
  ABI cells. Preserve its entry-value read before the first destination write;
  do not reject those useful cases merely because the address is unknown.
- Changed scratch writes have no in-window observer after their removal and
  no surviving observer of an intermediate definition outside the transaction.
  Ordinary RAM and the uninterrupted two-operation window are required. No
  hardware/opaque operation or call may be crossed.
- Plans carry snapshot generation and exact operation sites. Reconstruct the
  witness on application and replay physical proofs after final placement.

The current post-home dependency checker conservatively rejects an unknown
read while a disappearing fixed-home definition is unavailable. A fused
operation's indirect read instead observes the window-entry state. Add a
bounded typed witness for that exact original/replacement pair if needed by
[posthome.rs](../src/mir6502/rewrite/posthome.rs) and
[driver.rs](../src/mir6502/rewrite/driver.rs). It must validate the preserved
read prefix and internal definitions; keep the general unknown-read rejection
intact. A rule name, aggregate effect equality or an unverified escape flag
cannot supply this proof. If the witness is unavailable, retain both old ops.

## Code homes and integration

| Code home | Work |
| --- | --- |
| [calls.rs](../src/mir6502/materialize/calls.rs), [store_consumers.rs](../src/mir6502/materialize/store_consumers.rs) | Preserve existing producers; discover any consumers already yielding the adjacent pair |
| [ir.rs](../src/mir6502/ir.rs), [verify.rs](../src/mir6502/verify.rs), [printer.rs](../src/mir6502/printer.rs) | Add the typed selected form, placement/operand restrictions and readable printing |
| [effects.rs](../src/mir6502/analysis/effects.rs), [machine_values.rs](../src/mir6502/analysis/machine_values.rs) | Complete byte effects, incoming dependencies, register/flag clobbers and invalidation |
| [context.rs](../src/mir6502/rewrite/context.rs), [plan.rs](../src/mir6502/rewrite/plan.rs), [posthome.rs](../src/mir6502/rewrite/posthome.rs), [driver.rs](../src/mir6502/rewrite/driver.rs) | Generation-qualified witness, structural transaction and narrow ordered-read validation |
| [materialize.rs](../src/mir6502/materialize.rs), [mod.rs](../src/mir6502/mod.rs), [passes.rs](../src/mir6502/passes.rs) | Small final selector and independent feature control |
| [emit.rs](../src/mir6502/emit.rs) | Emit the selected carry chain and local label using existing tracked-emitter operations |
| [pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md) | Document operation ownership, preconditions, effects and rejected forms |

Place the selector after the existing final argument-staging selection, using
the complete accepted baseline. Keep earlier inlining, allocation and costing
identical between feature settings. Use a small module beside the existing
final selectors, without reorganizing the materializer. Audit exhaustive MIR
matches, home initialization, address-consumer visitors, remapping, cost
estimates and pre-emission verification when adding the selected form.

The call is outside the matched window. Discovery requires no scalar callee
signature or new machine-callee preservation assumption; normal conservative
call effects still participate in exit-state liveness. Ordinary result/store
uses may qualify whenever existing materialization exposes the same typed pair.

Introduce `enable_pointer_byte_plus_one_fusion`: false in conservative/default
configuration, independently controlled within peepholes, and initially false
in optimized configuration until qualification completes.

Apply one transaction per rebuilt snapshot with the shared post-home driver.
Each fusion removes one operation, so the original operation count plus one
bounds convergence. Search only adjacent pairs; there is no scheduling window,
CFG traversal or whole-program materialization trial per candidate.

Use existing routine code/home costing and exact load-file length. Require no
affected-routine growth, identical frames and declared/allocated zero-page
occupancy, no whole-image growth and a strict measured byte saving. Restore
cost-rejected routines, recheck combined layout and replay the physical witness.
Unknown emission/cost or failed validation retains the complete accepted
baseline. Bound costing to baseline, proposed batch and one combined repair;
record emission passes and additional materializations.

Report `pointer-byte-plus-one-candidate`, `selected`, `applied` and useful
blockers for shape, placement/overlap, incoming-read proof, removed definition,
live A/X/flags and final proof/cost. Count final applications only after costing.
Reporting must leave the image unchanged.

## Slice 1 Freeze the gap and establish the proof

1. Freeze current compiler/source/image hashes, raw/materialized MIR, listing,
   routine sizes, frames and zero-page facts. Preserve the existing Strcat
   audit as a historical control. Store new artifacts under ignored
   `build/pointer-byte-plus-one/before/`.
2. Add a neutral runtime-input fixture with a live caller. A suitable first
   case passes `p+p^+1` to an independent `Observe(CARD dst,src BYTE len)`
   callee. Add an ordinary result/store sibling to inspect applicability;
   do not assume it uses the same selected producer path.
3. Confirm the adjacent MIR pair with optimized NIR and identify the actual
   materialization phase. Test `+0`, `+2`, subtraction and plain arithmetic as
   controls retaining their existing strategies.
4. Add an inert typed proof query and rejection tests for physical overlap,
   incomplete definitions, live A/X/flags, stale sites and changed/observed
   intermediate definitions. Establish the bounded window-entry read witness.

Acceptance: one confirmed neutral miss, exact baseline arguments/results and
an implementable shared proof. Compiler output stays byte-identical.

Suggested commit: `Freeze pointer byte increment baselines and fusion proofs`.

## Slice 2 Add the selected MIR contract and emission

1. Add the new target form and update all MIR consumers and the pseudo-machine
   contract together. Enforce fixed-pair disjointness, complete non-wrapping
   words, legal Y offsets, supported phase and absence of scaled-Y consumers.
2. Implement the 21-byte candidate with existing emitter primitives. Keep all
   selection decisions in MIR6502 materialization.
3. Test effects independently against emitted reads/writes and clobbers,
   including the conditional high-byte repair and its local branch. Add
   verifier failures for malformed operands and focused printer coverage.
4. Execute emitted fragments against a host modulo-65536 oracle, comparing
   destination bytes, source-pair preservation, guards and stack balance.

Acceptance: the selected form is verifier-clean and machine-correct; default
selection remains off and existing outputs are unchanged.

Suggested commit: `Add verified MIR6502 pointer byte increment emission`.

## Slice 3 Select and cost the local fusion

1. Match the adjacent typed pair after normal materialization. Prove all
   changed machine state dead, final word equality and internal memory
   dependencies through the shared context and physical map.
2. Commit through the shared driver, with narrow ordered-read validation for
   the new witness. Missing facts select the old two operations.
3. Add independent feature-off/on integration tests, final routine/home/XEX
   costing, per-routine rollback, combined verification and placement replay.
   Test complete fallback on unknown cost and independent accepted/rejected
   routines. Keep optimized default disabled until slice 4 qualifies it.
4. Verify repeated application is stable and reporting off/on emits identical
   images. Retain preceding call, argument-staging and pointer-retention choices.

Acceptance: a neutral call saves bytes with unchanged arguments, result,
observable memory and storage capacity. Every rejected case remains compilable.

Suggested commit: `Select and cost proven pointer byte increment fusions`.

## Slice 4 Qualify boundaries and measure applicability

Use an independent host arithmetic oracle, emitted-instruction execution and
callee-entry VM observations. Batch complementary boundaries:

| Area | Cases |
| --- | --- |
| Arithmetic | All 256 pointer-low × 256 loaded-byte combinations on a safe page; high-byte and word-wrap boundaries `$0000/$0001/$00FF/$0100/$7FFF/$8000/$FF00/$FFFF`; bytes 0/1/127/128/254/255 |
| Machine state | A or X live into a successor/call, each live arithmetic flag, complete carry chain, Y setup, branch placement near page boundaries and balanced stack |
| Memory | Exact/partial physical pair overlap, separately named aliases, indirect targets at either pair/ABI cells, incoming destination contents, offsets 0/1/255 and pointer-plus-offset wrap, changed source-high dependencies and watched source-read count/order |
| Structure | Extra/edge uses, stale generations, two neighboring candidates, iteration bounds, offset 256 rejection and unsupported constants/operators/consumer shapes |
| Execution | Feature-off/on, raw/optimized NIR, both runtimes, two origins, actual LF/CRLF parsing and repeated runtime-populated calls |
| Existing behavior | Existing word arithmetic, pointer-byte argument setup, argument staging, scalar/predicate captures, protected indirect fields and the two Strcat parameter-home alias probes |

Avoid seeding an exhaustive pointer target over code or carrier bytes. Exercise
those aliases separately with internally consistent memory; do not silently
skip arithmetic values or corrupt the harness to manufacture an input pair.
Resolve parameter homes and routine boundaries in new TN probes from current
listing/map facts; the frozen audit's instruction offsets change after fusion.

Measure candidates, applications and blockers in neutral fixtures, TN/TNDBG,
existing pointer/argument fixtures, a string loop and representative graphics
code. Toggle only this new feature. Record final routine/XEX sizes, code and
reserved-storage extents, frames, zero page, source/image hashes, actual target
cycles, compile time and bounded costing/materialization counts.

If selected with unchanged surrounding code, Strcat's local body would change
from 125 to 120 bytes; verify the actual final result. Its 20-byte post-call
reconstruction and all outgoing argument stores remain controls. A neutral
five-byte local saving does not establish a five-byte whole-image saving or a
general improvement for other constants. Document actual fallback blockers
rather than widening scope to force a TN result.

Pin the new flag off in historical direct-comparison/argument-staging corpus
exporters when reproducing their frozen manifests. Keep their original images,
role tables and audit expectations intact. Put this feature's measurements in
a new directory; compare default TN execution with the new measured images.

After proof, execution and cost checks pass, enable the optimized default and
run final backend qualification. Commit compact results under
`docs/benchmarks/6502-pointer-byte-plus-one/`, keeping dumps/logs/images ignored.
Update this plan, the contract and documentation index with measured limits.

Suggested commit: `Qualify pointer byte increment fusion and record results`.

## Validation commands

Documentation preparation checks content and links only. Implementation adds
the proposed focused targets before using these commands:

```sh
cargo test --locked --lib pointer_byte_plus_one
cargo test --locked --test mir6502_pointer_byte_plus_one
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml --test pointer_byte_plus_one
```

Final qualification runs the affected backend's library/integration targets
and its emission/load sweep:

```sh
cargo test --locked --lib mir6502::
python3 - <<'PY'
from pathlib import Path
import subprocess
targets = sorted(p.stem for p in Path('tests').glob('mir6502*.rs'))
subprocess.run(['cargo', 'test', '--locked',
                *[f'--test={name}' for name in targets]], check=True)
PY
cargo run --locked --bin actionc-mir6502-sweep -- --emit-load fixtures/mir6502
```

Run affected VM targets for the new form and existing call consumers, including
TN execution and the frozen alias audit:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test pointer_byte_plus_one --test argument_staging \
  --test scalar_captures --test call_predicates --test call_predicate_qualification \
  --test register_copy_forwarding --test pointer_retention \
  --test tn_codegen_reproducers --test tn_dispatch
python3 tools/compare6502/tn/strcat_breakdown.py --check \
  --roles-output build/argument-staging/strcat-byte-roles.tsv
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test strcat_cost_breakdown -- --ignored --nocapture
bash fixtures/runtime/run-direct-action-word-arithmetic-args-vm.sh
bash fixtures/runtime/run-indirect-call-fields-vm.sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --lib indexed_byte_fixed_action_args_execute_through_the_vm_library
```

The indexed-byte selection-count script has a documented baseline zero-versus-
six discrepancy; preserve its independent execution assertion above and record
the discrepancy separately. Add listing/MADS round-trip checks for the changed
emission, including relocation of the new local branch at both origins. Keep
binary bytes exact while checking actual LF/CRLF host fixtures.

Under [AGENTS.md](../AGENTS.md), shared NIR, semantic lowering, verification or
printing contract changes additionally require:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

No such shared change is planned. Explain any intentional materialized-MIR
fixture updates as target selection changes; raw NIR remains unchanged.

## Completion record

| Slice | Status | Evidence |
| --- | --- | --- |
| 1 Baseline and proofs | Implemented | Frozen images and source/compiler identities; inert two-operation proof; neutral entry/read oracle |
| 2 Target form and emission | Planned | Add typed operation and qualify emitted arithmetic |
| 3 Selection and costing | Planned | Commit shared proof transactions with exact final qualification |
| 4 Boundary and corpus qualification | Planned | Enable optimized default after full scoped checks and measurements |

### Slice 1 evidence

The [baseline manifest](benchmarks/6502-pointer-byte-plus-one/baseline.json)
records the frozen compiler and all eight neutral/TN/TNDBG image controls.
The neutral fixture is 668 cartridge / 750 standalone XEX bytes at both
origins. `Forward` is 95 bytes, including the confirmed adjacent 26-byte
pointer-plus-byte and increment sequence. The ordinary `Address` return
sibling is 33 bytes and follows a different lowering path. TN/TNDBG retain
their preceding sizes and 125-byte `Strcat`.

The five-byte neutral call with plain later arguments did not expose this
producer path. The retained fixture uses two ordinary incoming pointers and
later arguments `q+1,q^`, exposing the existing source-ordered expression
consumer without changing it. Plus-zero, plus-two, subtraction and plain
addition remain independent controls.

The inert witness checks complete source definitions on every path, fixed
physical pair disjointness, legal offsets, discarded A/X/flags and intermediate
definition observations. A shared must-write query tracks public as well as
private home initialization without changing existing private-read diagnostics.
The neutral integration target passes both NIR modes/runtimes/origins. Its
VM target executes 960 calls through actual LF/CRLF parsing, checks independent
arguments/results and exactly-once ordered source reads, and matches the
frozen optimized-NIR images byte for byte. The sampled forward call takes
356 cycles including wrapper and actual callee; all dumps/logs stay under
ignored `build/pointer-byte-plus-one/`.
