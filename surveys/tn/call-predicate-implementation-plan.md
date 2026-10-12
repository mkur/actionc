# MIR6502 call argument captures across predicates

Status: Stages 0–2 complete, 2026-10-12; bounded capture selection enabled.

Implement priority 2 from the next compiler steps: reduce private argument
captures that survive a pure predicate's control flow. This is direction 3 in
the [historical TN audit](output-audit-2026-10-12/README.md). Preserve the
argument values established by source evaluation, their observable ordering,
canonical parameter captures and public call/result ABI.

## Objective and evidence

The motivating expression is:

```action
RETURN(Apply(cell,value,setting#0))
```

The [generic fixture](reproducers/captured-call-arguments.act) compares it with
computing the pure predicate into a local before the call. In the audit:

| Routine | Instruction bytes | Spill bytes | Parameter bytes | Local bytes |
| --- | ---: | ---: | ---: | ---: |
| `Forward` | 70 | 5 | 5 | 0 |
| `PredicateFirst` | 43 | 0 | 5 | 1 |

The source comparison demonstrates 27 instruction bytes and four net home bytes
of potential, plus 32 fewer measured cartridge cycles including the wrapper.
An isolated `SetTagged` source experiment saved 31 load-file bytes per MIR root.
These are diagnostic comparisons, not a compiler saving to promise or a reason
to rewrite production source. Existing publication/tag vectors do not exercise
`SetTagged`; add a direct wrapper vector before claiming a TN runtime benefit.

The current qualified `SetTagged` listing still copies the two earlier word
arguments into four spill bytes, materializes the predicate through a fifth
spill byte, then reloads them for the call. The earlier address-value work did
not remove this pattern. The starting compiler source is `a15f9f36`; its
[qualification report](address-values/stage5.json) pins the executable and roots:

| Cartridge root | Starting load-file bytes |
| --- | ---: |
| TN classic | 18,095 |
| TN MIR6502 | 16,802 |
| TNDBG classic | 21,586 |
| TNDBG MIR6502 | 20,539 |

Recheck the generic fixture with this compiler in Stage 0. Keep the historical
audit immutable and write new measurements separately.

## Scope and ownership

MIR6502 owns this work. Verified NIR already supplies typed argument values,
loads, compares, control flow, callable signatures and effects. SemIR/NIR and
their snapshots remain unchanged. A demonstrated missing shared contract
requires a separate slice and the repository's shared-contract checks.

Initially support direct calls, ordinary compiler-owned parameter/local homes,
BYTE and complete WORD inputs, and a small acyclic predicate region with one
continuation. Ordinary pointer arguments are captured word values; this does
not permit repeating a pointee dereference. Start with `BYTE # 0` normalization
and the existing five-byte Action argument frame. Admit other layouts only when
the same proofs and final schedule support them; retain fallback otherwise.

No inlining, application special cases, new allocator, global SSA conversion,
general code motion, loop propagation or interprocedural specialization is
included. Bitmap arithmetic and comparison-loop pointer reuse remain separate
directions. Classic is a comparison backend.

## Existing foundations and phase placement

Build on these components:

- [call plans](../../src/mir6502/call_plan.rs) and
  [call materialization](../../src/mir6502/materialize/calls.rs): typed arguments,
  canonical homes, overlap checks and final staging order;
- [comparison materialization](../../src/mir6502/materialize/compare_branch.rs):
  `expand_compare_value_consumers` splits a numeric comparison into true/false
  branches and a continuation carrying one BYTE result;
- [pre-home snapshots](../../src/mir6502/analysis/prehome.rs),
  [rewrite contexts](../../src/mir6502/rewrite/context.rs), reaching definitions,
  dominance and lane liveness: exact producer and use identities;
- [parameter availability](../../src/mir6502/analysis/param_availability.rs) and
  [post-home snapshots](../../src/mir6502/analysis/posthome.rs): register/home
  availability and final machine state;
- the [pseudo-machine contract](../../docs/MIR6502_PSEUDO_MACHINE_CONTRACT.md),
  [rewrite workflow](../../docs/MIR6502_REWRITE_WORKFLOW_PLAN.md) and maintained
  [rewrite inventory](mir6502-rewrite-migration-inventory.tsv).

The first capture selector belongs before `expand_compare_value_consumers`,
while the motivating loads, comparison and call are still in one typed block.
Replacing proven private captures with reads of their unchanged ordinary homes
lets existing comparison expansion preserve the earlier values through the
join without private word homes. Later materialization must confirm that ABI
staging does not overwrite those homes before reading them.

The Boolean carrier is a separate selection. Its true/false paths must establish
the same normalized byte at the call continuation. Reuse the comparison and
block-argument machinery; neither textual labels nor source syntax identify a
predicate region. Keep the ordinary comparison-result home as fallback.

## Required proofs

A captured argument and a deferred read are equivalent only when a structured
proof establishes the same bytes at the original evaluation and the call.
Record the original load site, storage identity, complete byte range, width,
definition lanes, call use and paths through the predicate region.

Require all of the following:

1. The exact producer dominates the call use, with valid reaching definitions
   for every required lane. Initially require one complete definition and one
   argument use; extra low/high-lane, edge or successor uses prevent removal.
2. The source is ordinary compiler-owned storage. Absolute-backed formals,
   hardware cells, fixed ABI storage, descriptor/pointer dereferences and
   unresolved backing retain their captures. Constants and typed storage
   addresses may use their existing pure materialization rules.
3. All paths between evaluation and consumption preserve the source bytes.
   Calls, helpers, machine blocks, stack/opaque operations, any source or
   uncertain alias write, and absolute/hardware/unknown-pointer reads are
   initial barriers. A call is a barrier even if a summary says no memory
   effects; a preserves annotation alone does not authorize motion.
4. Observable argument evaluation and memory snapshots stay in source order.
   Reading an unchanged ordinary home at staging time needs this proof; it is
   not general permission to evaluate a later argument first. Keep predicate
   operand evaluation, short-circuit behavior, multiplicity and effects intact.
5. The numeric predicate is a normalized BYTE 0/1 on every reachable incoming
   path. No missing definition, foreign predecessor, redefinition, loop or
   ambiguous join may be ignored. Other source Boolean conventions keep their
   existing semantics; do not infer normalization from a BYTE type alone.
6. Final physical source ranges do not overlap ABI writes or preparation
   scratch before their reads. Check both lanes and all writes in the actual
   staging schedule; distinct storage IDs alone do not prove disjointness.
7. Live A/X/Y and flags remain valid on all exits. A Boolean retained in a
   register must survive the join and staging. An ABI byte written on the
   branches must survive until the call without affecting intervening reads.
8. The call target, signature, widths, argument order, ABI homes, call count,
   effects and public result stores are unchanged. For the motivating frame,
   preserve the first WORD in A/X, second WORD in Y/`$A3`, Boolean in `$A4`,
   five canonical parameter bytes and the result at `$A0`.

Expose proof queries through the existing immutable routine snapshot/context;
extend that framework for memory availability across the bounded region.
Do not make recognizers perform independent suffix scans or build another
general data-flow engine. Keep logical storage proofs and final physical
confirmation distinct.

## Delivery sequence

Implement and commit each stage separately. Update status and small maintained
measurements only after its checks pass.

### Stage 0 Freeze acceptance and the current baseline

Reuse the generic fixture, current root images and existing VM instrumentation.
Record typed MIR before comparison expansion, the generated predicate join,
final code/homes, ABI staging, wrapper cycles and executed home traffic.

Extend semantics coverage with effectful later arguments, earlier-memory
mutation, fixed/hardware sources, one-lane writes, additional users and joins.
Test the original snapshots and evaluation order rather than merely comparing
two source formulations. Add a direct `SetTagged` vector checking return status,
bitmap guards, tagged counts and valid/out-of-range inputs in TN and TNDBG.

Acceptance: baseline semantics pass, current root sizes/hashes are pinned,
and the spill gap is reproduced in both runtimes with actual LF/CRLF sources.
Known compiler failures are recorded separately and fixed before enabling a
selector that depends on their behavior. No compiler optimization changes.

### Stage 1 Define argument snapshot availability

Add typed facts for a call argument's original snapshot and availability through
the small predicate region. Reuse CFG, dominance, reaching definitions, effects
and lane liveness. The first admissible sources are ordinary complete parameter
and local loads; the first region is an exclusive comparison followed by its
single call continuation.

Define explicit proof blockers for mutations, effects, extra uses, incomplete
words, foreign predecessors and uncertain storage. Add physical confirmation
queries for source/ABI/scratch overlap. Document the contract in the existing
pseudo-machine document. Keep production selection disabled.

Acceptance: positive and negative proof tests pass, stale-generation facts are
rejected, and the four root images remain byte-identical to Stage 0.

### Stage 2 Remove earlier private argument captures

Select the proven earlier BYTE/WORD argument loads before comparison expansion.
Bind their call operands to the unchanged ordinary homes using existing typed
value forms; remove only exclusive private captures. Keep the predicate and its
ordinary Boolean result carrier unchanged in this slice.

Use the transactional rewrite driver. Its validation must independently check
the original sources and widths, deferred reads, removed definitions and
unchanged call/evaluation sequence. Extend an effect delta narrowly if needed;
do not globally erase or discount call-operand memory reads. Register the family
in the maintained inventory and invalidate/rebuild affected analyses.

Enable only in optimized MIR6502 after final materialization confirms physical
disjointness and non-growing cost. A failed confirmation restores the original
candidate and its captures. The size comparison includes final code, ordinary
parameter/local/spill homes and the complete image.

Acceptance: admitted `Forward`/`SetTagged` forms no longer need the four earlier
word spill bytes; the existing Boolean carrier may remain. Canonical parameter
captures and ABI staging survive. Effectful, aliased or physically conflicting
forms execute correctly through fallback. Report code and homes separately.

### Stage 3 Carry the predicate into its ABI home

Select the exclusive normalized Boolean result for its assigned call home.
Initially target the fixed Boolean argument slot of the motivating frame.
The two branches may establish that slot directly only when all incoming paths,
slot lifetime, intervening accesses and final schedule are proven. Register
argument homes require their own all-path clobber proof; they may initially
retain fallback.

Keep one call at the original continuation. Do not duplicate the call into
branches, introduce a source local or suppress other Boolean uses. Integrate
with existing comparison/block-argument lowering. Any multi-block change must
apply atomically to a generation-checked routine candidate and verify the CFG;
extend the transaction boundary minimally if the existing single-block plan
cannot express it. Do not perform partially applied edits across the diamond.

Acceptance: the motivating admitted wrapper has no private argument spills,
retains the five-byte parameter frame, and improves final code-plus-home cost
over Stage 0 without worsening the whole roots. Exercise both predicate paths,
settings 0/1/2/255, complete WORD arguments and BYTE/WORD results. Boolean extra
uses, unexpected predecessors, live flags, slot aliases and staging clobbers
must reject safely. The source `PredicateFirst` local assignment stays intact.

### Stage 4 Qualify and document the integrated result

Compare all accepted and rejected forms with selection enabled/disabled through
the library configuration. Add an independent generic call layout, including
ordinary local sources and a non-tail call with work after the result. Exercise
other ABI placements as semantic fallback cases; broaden selection only where
the same proof and cost policy establish a win.

Run final affected-backend qualification and measure both complete TN roots,
classic comparison images and the direct `SetTagged` vector. Retain the existing
sort/tag/publication vectors to catch regressions; do not attribute their cycles
to a wrapper they do not execute. Preserve the completed address-value savings.

Record the compiler/source hashes, final code/homes/image sizes, scoped cycles,
executed operand traffic and remaining conservative cases. Exact cycle values
are observations, not brittle correctness assertions. No faster code-growing
variant is enabled without a separately agreed size budget. Unknown cost or
non-profitable layouts retain fallback.

Acceptance: generic and wrapper semantics pass, targeted code-plus-home cost
improves, both MIR roots have no load-size regression, and classic image hashes
remain unchanged. Document measured compiler results independently of the
historical 31-byte source counterfactual.

## Validation and resource scope

| Area | Cases |
| --- | --- |
| Values | BYTE and full WORD/pointer arguments; 0, 1, 255, 256, 65,535; both high and low lanes |
| Predicates | False/true normalization; 0/1/2/255 inputs; exclusive and extra uses; pure short-circuit forms and unsupported shapes |
| Snapshots | Later calls change earlier memory; direct/indirect alias writes; one-lane mutation; source-order instrumentation |
| Storage | Ordinary parameters/locals; absolute-backed formals; fixed/hardware cells; final ABI/scratch/source collisions |
| Control flow | Both diamond paths; foreign predecessor; incomplete join; successor uses; redefinitions; loops rejected |
| Machine and ABI | Live flags/registers; tail and ordinary calls; register/fixed homes; nested/indirect/runtime calls as barriers or fallback |
| Applications | TN/TNDBG `SetTagged` behavior, bitmap/count/status guards, unchanged existing provider/directory acceptance |
| Runtime and host text | Cartridge and standalone; conservative/optimized library configurations; actual LF/CRLF compilations |

Use focused MIR6502 library tests and the `mir6502_call_abi`,
`mir6502_materialization_gap`, `mir6502_rewrite_inventory` and `mir6502_fixtures`
integration targets during implementation. Extend `tn_codegen_reproducers` or a
focused call-predicate VM target for the snapshot/ordering cases. Include the
existing nested-call and ABI runtime tests when their scheduling paths change.

Final qualification runs all MIR6502 library/integration targets and the full
`tools/vm-runtime-tests` crate, including `tn_dispatch`, `tn_directory_model`,
`tn_mydos_directory` and address-value consumers. Run full-root LF/CRLF image
comparisons. No 65816/68k suite is required for MIR6502-only changes. Any shared
NIR, semantic lowering, verifier or printer change requires NIR snapshots, the
NIR sweep and full `cargo test` under AGENTS.md.

Keep small fixtures/reports under this survey and binaries/listings/logs in one
ignored target directory. Use one debug-info-free, non-incremental build cache.
Preserve the frozen audit, address-value final compiler and historical baselines
during cleanup. Preparation of this documentation requires content/link checks;
it does not start implementation or require rebuilding compiler suites.
