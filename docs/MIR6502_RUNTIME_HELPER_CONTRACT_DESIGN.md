# MIR6502 runtime and standard library contracts

Status: proposed, 2026-11-17.

The [implementation plan](MIR6502_RUNTIME_HELPER_CONTRACT_IMPLEMENTATION_PLAN.md)
delivers SArgs capture support first, followed by arithmetic helpers and selected
standard-library contracts, with focused qualification after each slice.

Complete the MIR6502 contracts for runtime helpers and standard-library calls
so native call composition and ZP residence can use their supported
implementations as certified call boundaries. Keep arguments, entry
requirements, guaranteed results and possible effects separate. Use the same
fact representation and consumers for both, while preserving their distinct
private-helper and public-library ABIs.

The [Huffman audit](benchmarks/6502-huffman-facts/README.md) found no named ZP
homes in either layout. Runtime helpers are a major admission barrier:
`ReadBit` uses right shifts, `ReadBits` uses wide shifts, and `BuildTree` uses
multiply and shift helpers despite being a source-level leaf. These operations
already have useful effects, but cannot supply the entry proofs needed by
their callers. Completing these contracts removes that particular barrier;
it does not by itself establish profitable ZP placement.

## Existing descriptions and missing guarantees

[Materialization](../src/mir6502/materialize/runtime.rs) already provides
`helper_abi_for`, `helper_args`, `helper_additional_results`, `helper_effects`
and `helper_return_effects`. `MirRuntimeHelperDecl` records a helper ID, target,
ABI, additional results and effects. `MirOp::RuntimeHelper` carries its physical
arguments and results. This design completes those descriptions rather than
introducing a parallel callee database.

There are several missing connections:

- [Native entry composition](../src/mir6502/analysis/known_callees/entry_inputs/acyclic.rs)
  and the underlying [entry producer](../src/mir6502/analysis/known_callees/entry_inputs.rs)
  reject every runtime helper. Machine entry liveness conservatively observes
  flags at uncertified helper boundaries.
- [Residence call checks](../src/mir6502/parameter_homes/calls.rs) reject callees
  containing helpers; [residence lifetimes](../src/mir6502/parameter_homes/lifetimes.rs)
  also reject a helper in the candidate's own expanded body.
- Helper result descriptions do not uniformly carry widths. A primary fixed
  result identifies a base address, while additional results have explicit
  widths. Guaranteed definitions must cover every result byte consistently.
- [Verification](../src/mir6502/verify.rs) checks declarations and several
  invocation fields against existing tables, but a complete contract must
  also check the primary result and the authority of the selected target.

MIR6502 owns these machine contracts. SemIR and NIR continue to own typed source
computation; this extension requires no source syntax, NIR form or semantic
lookup from the backend. The older
[runtime effect map](RUNTIME_HELPER_EFFECTS.md) remains useful background, but
its classic-codegen write tables do not define the richer MIR6502 contract.

## One shared description

Introduce a small MIR6502 helper-contract module keyed by `MirRuntimeHelper`.
Reuse existing home, width, register, flag and directional-effect types. Move
the existing match tables behind it; retain thin materialization accessors
where useful. Do not make analyses maintain their own helper whitelists or
infer contracts from names, emitted addresses or source signatures.

Library contracts use that same fact representation, populated from ordinary
routine analysis or a validated library implementation description. They keep
the existing interface/routine identities; do not add `SYS` members to the
private helper enum or duplicate their public declarations in Rust tables.

| Contract component | Meaning |
| --- | --- |
| Canonical argument lanes | Ordered physical byte lanes that setup must supply, including all lanes of a wide operand. |
| Entry requirements | Registers, individual flags and fixed bytes consumed before a guaranteed definition; distinct from the complete memory read footprint. |
| Result layout | Explicit home and width for every output, including the primary result and additional results. |
| Guaranteed return definitions | Result bytes/registers established on every normal-return path, conditional on satisfying entry requirements. |
| Possible register and flag changes | Values that must be invalidated; these changes never imply guaranteed definitions. |
| Memory observations | Independent possible-read and possible-write footprints, preserving Unknown separately in each direction. |
| Normal return effects | Machine and memory changes on paths that return to the caller; used for continuation preservation. |
| Stack and control behavior | Balanced return, may stop, unsupported behavior or no normal return; a stop path has no caller continuation. |

The description is the helper's intended interface. A proof for a particular
invocation also needs a validated binding to an implementation that satisfies
it. An unavailable proof is distinct from a valid contract with no extra inputs.
Existing effect-only knowledge remains usable by consumers whose proofs need
only those effects; it does not become a complete entry certificate.

### Inputs and memory reads

Argument lanes are mandatory even if a helper body happens not to observe an
operand on one path. Additional incoming requirements come from the supported
implementation contract. Never construct them by copying the entire read set.
For example, multiply can read scratch that it initialized internally; the
caller need not initialize all of that scratch before calling it.

The [call-fact distinction](MIR6502_CALL_FACT_CONTRACT_DESIGN.md) also applies:
reading existing program state does not create a helper argument. Conversely,
a literal scratch input remains required even if a source global happens to
occupy the same numeric address. Input/output overlap is legal: reads occur
before the corresponding guaranteed definitions in liveness transfers.

### Results and flags

Normalize results into explicit byte lanes through one shared projection. The
wide arithmetic convention is two word results at `$C4/$C5` and `$C6/$C7`.
All four bytes must participate in definition, interference and verification
queries. Neither a single-byte primary result nor a broad `$C0–$C7` may-write
set is a substitute for that layout.

A certified helper result is available after a valid call. It establishes no
particular constant, register-to-memory equality or flag provenance unless a
separate existing proof supplies that relation. If a zero-count shift passes
its input through, its input remains required. It does not thereby define
carry or establish N/Z from the result.

Initially export no guaranteed flag results and no new preserved-register
claims. Supply precise incoming flag requirements only where the selected
implementation has been checked; an empty set is a proof, not a default.
Keep conservative flag clobbers. Use the existing processor-mode ABI, including
its decimal-mode assumptions, rather than silently inventing an extra caller
obligation or extending the tracked flag model.

### Program stop and normal return

Treat Break/Error and division-by-zero failure as program stop under the
supported runtime contract. Their terminal paths do not need to preserve
registers, scratch or caller homes for a continuation that cannot execute.
Do not require a complete clobber analysis of the stop handler to certify the
helper's normal return. This applies equally to runtime helpers and library
routines that can terminate through those boundaries.

Use the existing distinction between `helper_effects` and
`helper_return_effects`. Keep full invocation observations for deletion,
reordering and observable-store checks. Add a normal-return projection for
residence and state that survives to the following instruction. A simple
may-stop fact and normal-return effects are sufficient; no general exception
or recovery analysis is required.

| Query | Required effects |
| --- | --- |
| Does a caller home survive until the instruction after the call? | Complete writes on normal-return paths. |
| Which results exist if the call returns? | Guaranteed definitions on every normal-return path. |
| Can the invocation be removed, speculated or reordered? | Full invocation effects, including possible stop and observations before it. |
| Can a preceding observable store be deleted or moved? | Full observations, including the stop handler's existing observation boundary. |

For example, if division uses a finite scratch set on success, Error's other
clobbers do not make every caller ZP home unavailable across that division.
The divide-by-zero check still executes even when the arithmetic result is
unused. Preserve the current observable-state rules from the
[runtime error contract](ATARI_RUNTIME_ERRORS.md#optimizer-contract-and-tests);
normal-return precision grants no new deletion of prior fixed or escaped state.

Generated division helpers already prevent continuation even if Error returns.
Use the terminal contract of the selected runtime for legacy Break/Error.
An arbitrary replacement that can resume the caller cannot inherit that
contract without evidence. Binding freshness must cover changes to this
control behavior as well as changes to normal-return clobbers. A helper with
no normal return has no result or preservation certificate to manufacture.

## Binding authority and validity

The [runtime resolver](../src/mir6502/runtime.rs) selects generated helpers,
resident cartridge entries or [linked runtime routines](../src/mir6502/standalone.rs).
Source runtime bindings can also supply replacements. A helper enum value
alone does not establish which implementation executes.

| Binding | Evidence needed for the new certificate |
| --- | --- |
| Compiler-generated body | Generator-owned helper identity and the actual body/relocations selected for this program, backed by contract checks. |
| Default linked runtime body | Runtime package selection and matching implementation closure, including private callees. |
| Default cartridge entry | Explicit resolver selection of the supported resident implementation and its validated contract. |
| User replacement or unrecognized target | Independent evidence for the complete contract; otherwise decline the new entry/residence proof. |
| Deferred target | No selected-implementation certificate yet; retain existing conservative behavior until resolution. |

Record binding provenance at resolution using the existing helper and routine
identities. Matching a numeric address or routine display name is insufficient.
Do not infer the default contract merely because an override has the same
calling shape. Unsupported replacements continue to compile under existing
rules, but gain no new preservation or definition guarantees. Their new proofs
must not borrow the default implementation's finite footprint.

Reuse the current program-scoped analysis and freshness model. Any entry fact
that consumes a helper contract depends on its declaration, binding and relevant
body/closure. Changing any of those invalidates the fact and its native
ancestors. This includes changing a helper without changing the caller body.
For resident code, the dependency is the selected runtime identity and trusted
contract. No persistent global proof cache or generic plugin registry is needed.

Contract validation must precede consumption of stronger facts. The verifier
checks canonical argument order, primary and additional result homes/widths,
effect consistency and resolved target evidence at the phase that requires
them. A malformed invocation is an IR error. An unsupported implementation is
an unavailable optimization proof. No unknown component may silently become
an empty input, output or clobber set.

### Validation after helper resolution

Today both ordinary materialization and
[placement replay](../src/mir6502/preparation.rs) run post-materialization
verification before `resolve_helpers`; residence validation participates in
that verification. Simply requiring a resolved certificate there would reject
every new helper-dependent candidate before it could be bound.

Make the boundary explicit: structural post-materialization checks still run
before resolution, including helper invocation shape and ordinary binding
checks. Helper-dependent residence obligations remain pending at that stage;
they are not successful proofs. Immediately after resolution, run mandatory
residence and helper-contract validation against that candidate's bindings.
Apply this sequence to baseline preparation and every replay path. No candidate
may enter later proof-consuming cleanup, final acceptance or emission with a
pending obligation. Retain all unrelated verifier checks and final physical
validation. The pre-resolution core gains no stronger helper facts from a
contract that will only be validated later.

## Integration with shared analyses

Use one validated helper query for both direct helper sites and helpers reached
through native callees. Adapt its result to existing machine/home transfer and
memory-summary types; do not disguise a private helper ABI as an Action call.

At a supported helper site:

1. Validate the invocation against the selected helper contract.
2. Observe canonical inputs and additional machine requirements. Propagate
   those obligations through actual setup and every CFG predecessor using the
   existing guaranteed-definition analysis.
3. Invalidate possible register, flag and memory changes. Apply only certified
   result definitions as definitions; a scratch clobber cannot initialize a
   source local or discharge an incoming value dependency.
4. Compose the full read/write observations and retain the binding dependency.

Extend the shared machine-entry transfer to accept certified helper reads and
definitions. The current native-call callback supplies observations only;
merely routing helpers through that callback would miss their result
definitions. Uncertified calls retain their conservative transfer. Keep
`PlannedCapture` restrictions unchanged; new composition uses `ActualEntry`.

A validated helper is a terminal contract dependency for native composition,
even when its implementation contains private runtime calls. Its normal-return
contract covers every path and dependency that can return; a certified program
stop ends that traversal. Full invocation observations remain available
independently. The compiler need not reclassify its internal machine blocks as
source-level Action leaf routines.

Continuation preservation uses complete possible writes on normal-return
paths, including scratch restored before return. Compose that projection
through native callees without importing terminal-handler clobbers. Reads
remain independent of entry supply, and Unknown invocation reads retain their
ordering/deletion restrictions. No caller cleanup is authorized just because
the helper returns a correctly typed value.

## ZP residence and transaction boundaries

Use the same helper query in `ResidenceCalls` and in direct helper lifetime
checks. A named home live across a helper must avoid its complete physical
write footprint on normal-return paths. Fixed-ZP and absolute spellings of a
byte alias. A helper result in an overlapping slot clobbers the old source
value; it does not define that source object.

If a source value is dead at the invocation, its lifetime needs no preservation
across that invocation. Canonical setup, entry requirements, observations and
the selected binding still need validation. Rejecting an unsupported helper
must not accidentally reuse the ordinary native dead-call fallback as a
helper certificate.

The complete caller proof includes setup, invocation and result extraction.
Private helper operands outside `$E0–$EF` do not prove that the enclosing
routine preserves that pool: its own temporaries and descendant working homes
can still occupy it. Keep per-object liveness, ownership and descendant
reservation rules, allowing reuse between independent call subtrees.

Consumers must explicitly request the normal-return projection. Do not replace
the general `MirKnownMemoryWrites` footprint with that narrower set for every
consumer. Existing full-effect queries remain conservative, while residence
and continuation state may use the certified projection. Its transitive
dependencies participate in the same freshness and rollback checks.

Retain the shared terminal transaction, final physical checks, whole-image
cost acceptance and exact rollback. Helper certificates join the existing
[fact-validity contract](MIR6502_CALL_FACT_CONTRACT_DESIGN.md#summary-validity-across-transformations).
A stale or weakened dependency requires the existing rejection/replay behavior;
fresh facts cannot justify retaining a rewrite made under invalid assumptions.
This work does not redesign the issue-8 conservative fallback.

## SArgs as parameter capture

Treat `SArgs` as a compact implementation of the ordinary entry copy from
argument lanes to parameter homes. The existing typed operation,
`SArgsCapture { first_param, byte_count }`, already supplies the destination
identity and extent. Its owning routine supplies the ordered parameter layout.
Use that information directly; no new IR form, descriptor interpreter or
general helper-analysis framework is needed for this slice.

For five BYTE parameters, the normal-return meaning is:

| Parameter home | Incoming value |
| --- | --- |
| `a` | A |
| `b` | X |
| `c` | Y |
| `d` | `$A3` |
| `e` | `$A4` |

Build one shared capture summary from the operation, its owning frame and the
selected SArgs contract. Record every incoming byte read and every parameter
byte defined on normal return, preserving the byte order of words and pointers.
Apply reads before definitions. The helper also writes fixed scratch and
clobbers registers/flags; these physical effects remain separate from the
parameter definitions. An incidental scratch write cannot initialize a caller
local. The Break state read remains program state, not an extra argument.

Keep the emitted helper call and three-byte descriptor unchanged. Return-address
handling stays inside the runtime implementation and emission; analyses consume
the capture summary. Preserve the existing contiguous frame, its ownership and
physical checks, and the supported binding contract. Unknown/custom helper
bindings do not gain capture guarantees merely from the SArgs name. Apply the
normal-return rule above to Break/Error without analyzing handler clobbers.

### Separate capture strategy from ABI knowledge

`MAX_DIRECT_PARAM_CAPTURE_BYTES` selects the current emission strategy: direct
stores for up to four bytes, SArgs for larger supported frames. It must not
also limit which complete argument signatures the fact layer can understand.
In particular, remove that threshold from actual-entry ABI completeness for
a validated SArgs capture. Retain type/frame consistency, supported ABI extents
and per-call canonical lane validation. A descriptor's count field alone does
not authorize a wider argument ABI or unsupported parameter types.

Use the shared summary in existing entry analysis, callee memory summaries and
residence checks. Replace their blanket SArgs rejection with this supported
capture case, retaining full invocation observations and normal-return writes
as separate queries. Read `$A3` and later argument lanes explicitly; recognizing
only A/X/Y would lose arguments needed by the capture. Parameter bytes become
defined at entry, so they do not reappear as unexplained incoming memory values.
Reuse current program-scoped freshness and final physical validation.

The first supported case is a caller keeping a ZP value across a routine whose
entry uses SArgs. That callee must still have a supported body, and its complete
normal-return write footprint must avoid the live caller home. Include capture
scratch, the parameter frame, body temporaries and descendants. The presence
of SArgs alone must no longer disqualify the callee.

Keep that callee's captured parameters in their existing contiguous frame.
Choosing new homes for those parameters, widening its own placement eligibility,
and replacing capture with scattered stores are separate changes. This gives
the fact layer accurate parameter capture without changing the calling
convention or adding another placement mechanism.

### Focused capture checks

Use a five-byte mixed scalar/pointer frame to check every incoming and captured
lane, plus a four-byte direct-capture control. Check a caller value live across
SArgs, a collision with its scratch, and a nested native wrapper. Keep malformed
frame/count and unsupported-binding negatives. Reuse the existing capture
emission and cartridge/standalone VM controls to confirm unchanged descriptor
bytes and parameter values. These checks are sufficient for this slice; a full
library audit or new test matrix is not a prerequisite.

## Standard library coverage

The standard library belongs in this design from the start. Coverage includes
the [public SYS interface](../embedded/modules/sys.act), its
[cartridge](../embedded/bindings/sys-cart.act) and
[standalone](../embedded/bindings/sys-standalone.act) bindings, and ordinary
embedded Action modules such as
[MATH.INTEGER](../embedded/modules/math/integer.act). Constants-only modules
need no call contract. This scope concerns MIR6502; other targets retain their
own physical ABI and machine-effect contracts.

### Keep interface meaning and implementation facts separate

`SYS` already owns public parameter and result types. Its binding mechanism
selects an absolute entry or a linked implementation, and validates the linked
implementation's signature. Reuse this mechanism. A signature proves how to
call a routine, but does not prove its clobbers, memory observations or result
definitions on every return.

| Implementation | Contract source |
| --- | --- |
| Ordinary Action library routine | Existing analysis of its actual MIR body and descendants; library membership grants no special purity or preservation. |
| Embedded assembly implementation | Explicit compiler-owned implementation contract, checked against its body and complete runtime closure; reuse bounded machine analysis where supported. |
| Resident cartridge entry | Explicit contract for the selected implementation, connected to the existing interface binding. |
| OS, hardware or replaceable vector boundary | Independently supported boundary contract, otherwise conservative effects and unavailable optimization guarantees. |

Permit reviewed manual contracts for assembly instead of requiring a general
machine-code theorem prover. Store them with the implementation/binding
metadata and resolve them into the same typed MIR facts used by helper
contracts. Existing annotations may provide individual facts, but a clobber
annotation alone does not certify entry inputs or guaranteed results. No new
user-facing annotation syntax is required for this work.

Source-visible effects belong to SemIR/NIR under the existing architecture;
physical registers, flags and scratch belong to MIR6502. Consume the interface
facts carried into MIR. Do not add an optimizer lookup into SemIR or use a
display name to recover a lost signature.

### Preserve identity through runtime selection

[Interface resolution](../src/mir6502/interfaces.rs) rewrites references and
removes declaration-only routines. Retain a typed association between the
resolved call, original interface signature and selected binding before that
information disappears. Qualified and compatibility-prelude names must reach
the same contract. A user routine with the same name must not inherit it.

An address alone is insufficient even within the supported cartridge: `Peek`
and `PeekC` share `$A767` but expose different result widths. Keep the
interface-specific result contract while covering the selected entry's full
physical effects. Do not transfer a standalone clobber set to a cartridge
implementation merely because they implement the same public function.
Cartridge mode also links some modern `SYS` implementations from source, so
choose evidence per binding, not from the runtime mode alone.

Contracts for linked assembly cover jumps, private calls and fallthrough, with
normal-return coverage ending at a certified program stop.
For example, the [Zero entry](../corpora/action-runtime/extracted/SYSBLK.ACT)
falls through into `SetBlock`; its contract cannot stop at the short named
entry body. The existing runtime link manifest supplies closure identities,
not an effect proof. Body, dependency, ABI or binding changes invalidate the
contract and its callers' facts, just as for helpers.

`SYS` interface resolution occurs before materialization, whereas arithmetic
helper resolution follows the core. Validate each library certificate once
its selected implementation is available, then revalidate any body-dependent
facts after transformations. Use the common freshness and transaction rules;
do not force all providers into the helper's later resolution phase.

### Model caller buffers and observable state

Library operations need to distinguish fixed machine scratch, existing global
or OS state, and accesses through supplied addresses. For example, `SCompare`
reads string contents; `SCopy` reads its source and writes its destination;
`Zero` writes its destination. Each still has its own entry, scratch and result
effects. These are required categories to audit, not complete clobber maps.

Represent a proven buffer access by its argument identity and supported extent,
alongside fixed effects. Instantiate it using captured typed call arguments
and existing storage/ownership facts. If the extent or target is unknown,
retain that uncertainty in its direction. A dynamic buffer access must not be
silently omitted to obtain a finite scratch-only footprint. Do not assume
source and destination are disjoint or infer bounds from a declaration alone.

Reuse the [unexposed-storage rule](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md):
ordinary pointee accesses do not automatically expose every compiler-private
home. Such exclusion requires the existing ownership proof and a certified
ordinary-pointee effect; opaque assembly or unknown absolute access cannot
claim it. Address-taken objects remain observable. Any consumer that cannot
represent fixed effects together with buffer uncertainty keeps its existing
conservative answer until that projection is supported. Do not introduce a
separate library-only alias rule.

`Peek`/`Poke` families take explicit numeric addresses and retain physical
observations, including possible hardware access. A known count does not turn
an arbitrary address into private storage. For I/O, graphics and device calls,
reading OS state creates a memory observation rather than an extra argument.
Retain I/O and ordering effects even when the result is unused or some registers
and ZP bytes are preserved. Reaching an unsupported OS vector, handler or
callback prevents claiming a complete transitive footprint for paths that can
return. A certified Break/Error stop does not block a normal-return contract.

### Deliver coverage in small groups

Share the contract machinery immediately, then qualify library families in
bounded groups alongside the arithmetic-helper work:

1. Use an ordinary Action arithmetic wrapper as the analyzed-library control,
   and certify a small assembly string-reader boundary such as `SCompare`.
   This exercises helper composition, a public result and indirect reads.
2. Add `SCopy`, `Zero`, `SetBlock` and `MoveBlock` with explicit buffer effects,
   complete scratch writes and per-site overlap checks. Larger argument frames
   or unsupported entry capture remain independent admission blockers; a
   manual effect map cannot bypass them.
3. Extend conversion, I/O and device families after their returning helper and
   OS dependencies have contracts; terminal Error paths use the stop rule.
   Keep unsupported entries explicit rather than granting the whole library
   a blanket known-effects classification.

Maintain an inventory keyed by interface and selected implementation: entry
support, result guarantees, directional effects, closure dependencies and the
reason any proof is unavailable. Reuse the existing fact-reporting path. This
allows library coverage to grow without a new optimization layer for each
routine or a requirement to complete every library contract before using one.

## Initial coverage and separate work

Start with the arithmetic used by Huffman, plus the corresponding compact
multiply helper. The following table records the existing calling layouts;
promotion to a complete certificate requires checking each selected binding.

| Helpers | Required argument lanes | Guaranteed result to certify |
| --- | --- | --- |
| `Lsh`, `Rsh` | A/X operand, `$84` count | A/X word |
| `Mul` | A/X operand, `$84/$85` operand | A/X word |
| `MulByte` | A and X byte operands | A/X word |
| `Lsh32`, `Rsh32`, `Mul32` | `$82–$85` operand, `$C0–$C3` operand/count | `$C4–$C7` |

Keep existing conservative effect ranges until a separate verified refinement
justifies narrowing them. Shared shapes do not imply identical entry flags,
clobbers or target evidence across helper families.

Include division/remainder and `SArgs` as follow-on slices of this design.
Their possible program stop is not a reason to defer their contracts. Division
uses its existing normal-return scratch effects and the appropriate result
layout, while retaining the divide-by-zero observation boundary. `Fault`
contributes a terminal boundary and has no normal result.

Implement [SArgs as parameter capture](#sargs-as-parameter-capture) through the
existing structured operation and shared analyses. It can be delivered as a
narrow slice without waiting for the entire helper/library catalog. Its frame
and emitted capture remain unchanged.

Keep the following Huffman findings separate:

- `ReadBits` has unsupported direct result metadata for `LONGCARD`, which also
  suppresses its otherwise supported parameter ABI. Private wide-helper
  results do not repair the public Action call ABI.
- Caller admission still restricts multiply, shifts, some pointer arithmetic
  and global address formation. A helper contract does not prove those
  independent transformations safe.
- Local-only leaf discovery, byte-parameter forwarding and ordinary-global
  physical-overlap precision retain their current limits.
- `ReadHeader` already uses the full temporary ZP pool. Better facts need not
  produce a smaller image after competing temporary placements are accounted for.

## Focused qualification

Contract tests should cover distinct facts rather than a large product of
equivalent configurations:

- Check canonical lanes and exact result widths, especially all four wide
  result bytes. Reject missing/reordered inputs and malformed primary or
  additional results at a later repeated call as well as the first call.
- Check an initialized scratch read does not become an incoming requirement;
  genuine register/fixed-byte inputs still propagate until actual setup
  defines them on every path. Conditional setup and may-writes are negatives.
- Exercise zero-count and boundary shifts, zero/max operands and both result
  halves. Verify flags are not invented from result availability. Use existing
  arithmetic VM tests to check values and documented clobber boundaries;
  unchanged bytes in a sampled run alone do not prove a preservation contract.
- Check a live caller home outside helper scratch, an overlapping home, a dead
  value at the helper and a two-level native wrapper. Check read observations
  survive even when no value needs preservation.
- Check an override, unresolved target and changed helper body/binding cannot
  reuse a default or stale certificate, including changed stop behavior.
  Validate generated/linked bodies and supported cartridge bindings against
  their respective evidence.
- Check successful division preserves a live home outside its normal scratch;
  zero division still stops with an unused result and retains prior observable
  state. Reuse the existing returning-Error-handler control. Check `SArgs`
  defines its complete frame on normal return, preserves a disjoint caller
  home, retains the Break check and rejects an invalid descriptor or unsupported
  overlap. No full Error clobber map is required for these continuation proofs.
- For library calls, check qualified/prelude identity, user-name shadowing,
  shared-address entries with different result widths, and independent cart
  and standalone bindings. Include a fallthrough dependency and a changed
  descendant to exercise contract invalidation.
- Exercise library buffer reads/writes with live private homes, exposed objects
  and overlapping buffers under each routine's supported semantics. Keep raw
  numeric-address and observable I/O controls conservative. A correctly typed
  return or preserved register must not erase the invocation's side effects.

Then repeat the Huffman inventory and its existing cartridge/standalone VM
controls, including their LF/CRLF paths, plus a TN size/home comparison as an
independent control. Report certified entries, remaining rejection reasons,
named homes versus temporary ZP, packaged bytes and any fallback. Qualify each
library group with its affected MIR6502 integration/VM targets in both runtime
bindings where supported. A newly certified entry with unchanged image size
is a valid contract improvement. Run focused MIR6502 checks; documentation
preparation itself requires only content and link validation.
