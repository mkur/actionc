# Native 65816 processor-state tracking

Status: proposed design, 2026-09-21, based on main `956facf`. This note builds on
the implemented [local accumulator forwarding](MIR65816_LOCAL_ACCUMULATOR_FORWARDING.md)
and the existing 6502 tracked emitter. It specifies a direction and staged
acceptance criteria; it does not claim the broader tracker is implemented.

## Purpose and boundary

Introduce one instruction-aware owner of native 65816 state facts. Use those
facts to prove local instruction substitutions without changing Action!
semantics, ABI placement, allocation or observable memory accesses. The first
integration must reproduce current machine code exactly, including current
adjacent-word forwarding. Broader forwarding follows as a separately measured
change.

MIR65816 selection owns target strategy: operands and physical homes, byte/word
expansion, register use, addressing, helpers, ABI sequences and eligible
optimizations. The tracker owns the consequences of the concrete instructions
actually emitted and answers explicit proof queries. It must not look into
SemIR, parse printed IR, rediscover aliases from names, allocate registers or
change the selected addressing strategy.

Keep [physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md), image v3, the
[o65 profile](MIR65816_O65_PROFILE.md), source accesses, temporary stores and
homes, stack checks and interrupt headroom unchanged in the initial stages.
Exec816's compiler pin and loader qualification are separate. This is support
for the [code-quality roadmap](MIR65816_CODE_QUALITY_PLAN.md), especially width
handling and later register use; it does not combine those optimizations.

## Reuse from the 6502 implementation

Use the current code as the implementation reference:

| Existing component | Reuse in 65816 | Required adaptation |
| --- | --- | --- |
| [TrackedEmitter](../src/codegen/tracked_emitter.rs) | A single facade couples instruction emission with state updates; explicit compiler barriers; test snapshots. | Typed native widths, stack/bank addressing and existing symbolic fixups. |
| [NativeProcessorState](../src/codegen/native_state.rs) | Separate register, flag and memory facts; conservative invalidation; proof queries such as value equality plus matching N/Z. | This is a **6502 byte tracker**, despite its name. Do not widen its u8/u16 fields and assume native semantics follow. |
| Native-state tests in that file | Unknown-value rejection, dependency invalidation, alias chains, register writes, memory mutation, calls and labels. | Add partial lanes, overlapping stack/DP ranges, mode changes, reused homes and S movement. |
| Tracked-emitter tests in that file | Assert actual bytes alongside state; a barrier must prevent an otherwise valid optimization. | Cover native encodings, raw and optimized MIR, relocation and interrupted execution. |
| [MIR6502 emission](../src/mir6502/emit.rs) | Verified MIR feeds concrete instruction helpers; volatile/order barriers remain explicit. | Retain MIR65816's existing selector and allocator contracts. |

The archived [tracked-emission boundary note](archive/implementation-plans/mir6502/MIR6502_TRACKED_EMISSION_BOUNDARY.md)
provides the ownership model. Its proposed names and APIs are historical; the
current `TrackedEmitter` and `NativeProcessorState` above are authoritative.
The older [codegen state model](../src/codegen/state.rs) is useful background
for storage dependencies, but is not the tracker wired into MIR6502 emission.

Preserve the distinction already visible in 6502: ordinary CMP emission and
`emit_cmp_imm_for_z_branch` have different proof requirements. A consumer that
needs only Z must explicitly establish that narrower requirement. Do not
generalize a Z-only omission to users of C/N/V. Likewise, tracking a constant
does not itself authorize removal of a memory read or write.

Keep the two processor implementations separate initially. Share design rules
and test scenarios; defer a generic tracker or shared instruction framework
until two proven implementations demonstrate a useful common interface.

## Current native state and proposed structure

Today [Code](../src/mir65816/emit/code.rs) tracks A8/A16/unknown and clears width
knowledge at every label. [ResidentWord](../src/mir65816/emit/accumulator.rs)
certifies one private word in A16 and its N/Z, using TempId, exact stack slot,
zero transient displacement and an unchanged byte/label cursor.
[Builder](../src/mir65816/emit/select.rs) separately maintains transient stack
displacement. There is no instruction-level X/Y, carry, DP-content or join
analysis. Current safety comes partly from rejecting every intervening
instruction, including ones which happen to preserve the needed facts.

Suggested private modules under `src/mir65816/emit/`:

| Module | Responsibility |
| --- | --- |
| `state.rs` (new) | State facts, invalidation, instruction transfer functions and proof predicates; no bytes or source semantics. |
| `tracked.rs` (new) | Concrete instruction facade; precondition checks, encoding and application of the corresponding effects. |
| `code.rs` | Byte/label/fixup storage and final proof metadata. Raw writes available only behind the facade once migration completes. |
| `select.rs`, `accumulator.rs` | Checked target selection and producer/consumer eligibility. Request proofs and emit the chosen sequence. |

The facade owns `Code` and `State65816`. Move width knowledge into that one
state owner rather than keeping two independent width caches. Expose finalized
`Code` to linking and qualification as today. Keep `Code.mir_spans` and typed
fixups nonserialized and independent of optimization correctness.

A modeled instruction has a concrete opcode/addressing form, checked operand
extent and explicit width requirements. Its encoding and transfer function
must be selected together. All opcode-writing paths, including guards, calls,
pointer setup, edges and teardown, pass through this boundary. An exhaustive
private instruction representation or equivalent typed helpers can enforce it;
there is no need for a new public IR or a general CPU emulator.

Separate encoding preconditions from optimization permissions. Existing fixed
sequences, including internal guard labels, need checked width/stack contracts
even where the current local width cache is unknown. Choosing a sixteen-bit
immediate encoding is not evidence that the CPU is in A16. Validate those
sequence contracts during migration; their proof must not silently enable new
width omissions before the separate block-entry optimization is qualified.

Preserve transactional preflight: validate all operands, destinations and
fallback conditions before changing bytes or facts. If the selector falls
back, it must not leave speculative facts. The tracker answers a proof query;
only the selected emitted instructions advance its state. An omitted load
leaves state unchanged because the query proved its effects already hold.

## Facts and identity

Unknown means no proof. Two unknown values do not establish equality. Each
opaque produced value receives a fresh identity, scoped to this emission
region; copied values may share it. Constants are width-qualified. Use stable
MIR identities when binding a produced value to a temporary, with the exact
byte range and current contents generation of its allocated home.

Do not use mutable names such as “the current A” as permanent value identities.
The 6502 implementation conservatively invalidates dependent register aliases.
Prefer immutable value tokens in the native design: after A changes, a captured
copy in a home or X still names the old value. Unknown inputs may receive fresh
opaque tokens, but never one shared token standing for every unknown input.
Do not carry tokens through a backedge by reusing a TempId or source name.

| State | Representation and initial policy |
| --- | --- |
| A | Complete word fact with explicit width; allow a low-byte fact only when its upper-byte knowledge is separately represented. Initially discard full-word eligibility on width transitions or partial writes. |
| X/Y | Width-qualified value facts. Model clobbers from the outset; using them to omit instructions is a later slice. |
| N/Z | `NZFor(value, width)` or unknown; known bits may supplement this. Matching bits of A without matching flag provenance is insufficient. |
| C/V | Separate unknown/known facts, optionally producer provenance for a checked compare/arithmetic sequence. No flag-liveness inference in the first optimization. |
| Modes | Known/unknown E, M and index-width bit; decimal flag named distinctly from direct-page register D. |
| I | Preserved entry token unless an explicitly declared IRQ-state operation changes it. Do not assume callers start masked or unmasked. |
| Address environment | DBR knowledge and a symbolic current-domain D identity. Symbolic data/code targets retain typed relocations, not assumed linked addresses. |
| Stack | Invocation-relative body-S anchor and checked transient displacement. Keep its authoritative accounting synchronized with emitted pushes, pulls and S updates. |
| Private memory | Bounded map from canonical byte ranges and contents generations to captured value tokens. Start with exactly two-byte stack temporaries. |

An immutable value token is a snapshot of bits, not a promise that its source
memory will retain those bits. A write invalidates equality with overlapping
memory and any facts defined in terms of its current contents. It need not
destroy an independently captured register value. Initial implementations may
discard more facts; they must never keep an invalid relation.

Bound storage by the routine's allocated homes and the supported register
facts. Do not build expression trees or an unbounded symbolic interpreter.
Fresh tokens are identities, not an invitation to reimplement NIR folding.

The initial reload query must prove all of the following together: selected
two-byte private TempId and exact allocated range; unchanged home contents;
the same complete value in A; A16 with word-wide matching N/Z; the permitted
stack state; and no crossed barrier. Complete operand validation remains a
separate prerequisite. Keeping an A fact after N/Z changes is useful knowledge,
but does not make this query succeed. Neither C/V liveness nor equal unknowns
can substitute for the missing proof.

## 65816-specific rules

The hardware reference is the [WDC W65C816S datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf),
revision March 13, 2024, especially sections 2.4–2.11, table 5-5 and sections
7.10, 7.18, 7.20–7.23. M preserves the accumulator's hidden upper byte; setting
index width to eight bits clears the high index bytes. Register transfers use
destination width; TCS/TSC/TCD/TDC require their special full-width rules.
MVN/MVP affect DBR. These facts require instruction-specific effects.

Compiler policies derived from those distinctions:

- `REP #$20` is not zero extension. After an A8 write, returning to A16 cannot
  manufacture a known complete word or word-wide N/Z. Initially require a new
  full-word producer; defer combining low/high lane facts.
- REP/SEP effects depend on the complete mask. Track or invalidate each changed
  status bit; do not interpret all REP/SEP as just an M change. A redundant
  mode request may emit nothing only when all requested effects already hold.
- Loads and arithmetic establish N/Z at their actual width. CMP/CPX/CPY and
  transfers can invalidate an A-related N/Z proof while leaving A unchanged.
  Arithmetic facts require the appropriate decimal-mode proof.
- PLP, RTI, XCE, XBA, block moves and unsupported transfer combinations initially
  use conservative effects. Unknown E/modes cannot be repaired merely by
  forgetting facts or issuing REP. Unsupported executable paths require an
  existing qualified adapter or a diagnostic, not a newly assumed ABI state.
- A call/result lane, pointer's final overlapping word and complete word temp
  are different facts. Do not forward a three-byte transfer's last word as a
  two-byte TempId. Preserve the ABI's explicit high-bit guarantees for results.

## Memory, stack and DP ownership

Canonical stack identity is invocation/frame plus physical byte range, related
to the selected TempId and allocation. An encoded `d,S` operand alone is not
an identity: changing S changes what it addresses. A reused slot does not make
two temporaries equal. Keep contents generations per overlapping range; a
one-byte write invalidates a containing word proof, and a word write invalidates
overlapping byte/word proofs.

Begin with zero transient displacement and clear forwarding facts on all
reserve/release/push/pull/S changes, as today. Later preservation across known
S movement needs a stable body-S anchor, interval checks and exact updates to
displacements. Unknown S writes invalidate stack-address relations. Every
elided load still runs the normal complete extent check, including the last
valid word displacement 254 and rejection at 255. No facts authorize a red
zone, smaller guard or unaccounted temporary stack use.

Only non-addressable compiler homes qualify initially. Incoming parameters,
addressable locals, globals, absolute aliases, indirect/indexed memory and
volatile/device accesses retain their source reads and writes. A direct load
may still establish the existing captured-private-word fact after its retained
store; that does not cache its source. Unknown aliasing or source-memory
effects remain barriers to the initial optimization.

Later DP facts must identify the current execution domain, owned offset range
and contents generation. They are not global zero-page addresses. The full
$00–$3F scratch region is call-clobbered, and pointer/result/scratch aliases
overlap within it. Domain metadata and reserved bytes are not allocatable.
Consult the allocated homes and selected operation effects; absence of an
obvious DP instruction does not prove a helper preserves scratch. Tracking
existing DP contents and allocating new scalar DP homes are separate changes.

## Instruction effects and barriers

The following is the initial effect policy, not a list of new optimizations:

| Operation | Required state treatment |
| --- | --- |
| Checked LDA16/private STA pair | Establish the actual produced value, N/Z and exact home contents; publish only through the existing producer whitelist. |
| Native ADC/SBC | Consume selected operands and carry; produce a fresh result and matching N/Z. C/V become result facts or unknown. Preserve retained destination store. |
| CLC/SEC | Change C only. Potentially retain A, its home relation and N/Z. Removing CLC/SEC itself is outside the first optimization. |
| CMP/CPX/CPY, LDY or flag-setting transfer | Apply actual register effects and replace the relevant N/Z/C proof. A alone cannot justify dropping a subsequent LDA. |
| Proven disjoint private store | Update destination contents without changing CPU flags; retain unrelated facts only after range checks. Initially keep all source-memory stores as barriers. |
| Width transition | Update modes and invalidate affected word/lane proofs conservatively; never infer widening of N/Z. |
| Guard, outgoing setup, edge assignment or stack adjustment | Retain existing code and clear optimization facts at the sequence boundary initially, even if some component instructions are modeled. |
| Volatile/ordering barrier | Forget optimization facts explicitly, even when no bytes are emitted; preserve only independently justified machine invariants. |
| Unmodeled/raw executable bytes | No value proof may survive. If mode/S/D/DBR effects are unknown, those facts are also unknown; continuation requires a checked postcondition. |
| Label/jump/return | Clear local optimization facts; no inheritance from the preceding layout block. Bind labels through the facade. |

Do not let migration code bypass effects with a raw encoder reference. Known
but not yet precisely modeled instructions can use a reviewed conservative
effect descriptor. Its preserved mode/environment facts still need an opcode
proof. A byte cursor alone cannot protect broader forwarding once intervening
instructions are allowed; keep a coverage assertion that every emitted
instruction was modeled or explicitly treated as a barrier.

For calls, consume the existing [native ABI contract](../src/mir65816/abi/mod.rs)
and typed interface metadata. Direct, indirect, recursive, runtime and assembly
calls initially clear A/X/Y, C/N/Z/V, memory relations and all DP scratch facts.
They remain ordering barriers. After a normal return, reestablish only guaranteed
boundary facts: native mode, A/X/Y width, binary arithmetic, DBR zero, preserved
domain D, specified I effect and checked stack balance. Values in result lanes
are fresh results, never pre-call register contents; keep current result
marshalling outside the producer whitelist initially.

A helper without a narrower qualified effect contract takes that same
conservative treatment. Inline helper sequences need effects for their actual
instructions; “no JSL” does not mean “no clobbers.” Do not infer preservation
from a helper name or current incidental assembly. Narrower contracts, if
introduced later, belong to target strategy and require independent tests.

## Control flow and preemption

Initially every label forgets values, flags and local width knowledge exactly
as today. Routine entry knowledge comes from the ABI, not from the previous
emitted routine. Calls may establish only their declared return facts.

A later width-only analysis may attach checked entry contracts to MIR blocks.
Compute them over all predecessor edges, including loop backedges, before
emission; meet retains a fact only when every incoming path proves it. Unreachable
and unknown are distinct. Internal guard/compare labels are not automatically
MIR entries and cannot inherit a blanket A16 assumption. Verify the actual
emitted exits honor each contract. Keep value/flag facts empty at joins during
this stage. Branch shortening and jump removal remain separate layout work.

Value propagation across blocks needs additional dominance/edge-parameter and
physical-copy proofs, with convergence independent of emission order. It is
deferred, as are loop register allocation and home coalescing. A tracker records
register contents; it does not reserve them against subsequent selector scratch
use. Any later allocator must supply and verify those live-register constraints.

Asynchronous IRQ/NMI suspension is governed by the existing
[context contract](MIR65816_CONTEXT_INTERFACE.md): full CPU restoration and
separate persistent task/IRQ DP domains. Facts about captured registers and
owned private storage may remain true after resume under that contract.
This does not make shared source memory stable across interrupts. Calls such as
yield still follow ordinary call barriers. Do not mask interrupts to make
tracking easier or reduce stack headroom; qualify every newly extended live
register/flag interval in both task domains and both incoming I states.

## Staged implementation and acceptance

1. **State model and effect tests.** Add the private model and concrete effect
   vocabulary. Port the 6502 unknown/alias/barrier scenarios and add native
   width/range/stack cases. No instruction selection changes.
2. **Byte-identical integration.** Route native emission through the facade,
   consolidate width knowledge and represent the existing ResidentWord proof
   in the new state. Retain the adjacency cursor and every existing eligibility
   gate as policy. Preserve all bytes, diagnostics, frame maps, fixups and proof
   metadata behavior. Audit every encoder mutation and prohibit untracked writes.
3. **Broader local private-word forwarding.** Freeze candidate sites and dynamic
   counts first. Allow only reviewed intervening effects preserving the exact
   value/home relation and N/Z, for example CLC/SEC or proven disjoint private
   stores. Keep current producer/consumer classes, zero stack displacement,
   stores/homes, source barriers and no flag-dead exceptions. Enable each useful
   instruction class with focused negatives and measured machine execution.
4. **Checked block-entry widths.** Add the separate CFG proof described above;
   use it to remove only redundant width setup. Do not also change branches,
   copies or register allocation. This supports roadmap step 3.
5. **Local X/Y and existing DP facts.** Add instruction-specific lane and scratch
   coverage, then independently selected uses. Scalar DP allocation, cross-block
   values and loop residency still require their own plans and qualifications.

Stages 1–2 are the recommended first implementation slice. Their acceptance
criterion is a trustworthy, centrally enforced state boundary with identical
output, not a code-size gain. Stage 3 needs a fresh measurement-based plan;
there is no evidence yet for a numerical saving beyond current forwarding.

For a symbolic A16 example, a private `STA t,S; CLC; LDA t,S` can potentially
omit the LDA when the retained store and N/Z proof still hold. Replacing CLC
with LDY does not satisfy the same proof. These illustrate selection rules,
not measured corpus opportunities or new source-language behavior.

## Validation and evidence

Use [current forwarding qualification](abi/action65816-accumulator-forwarding-qualification.json)
and its [saved corpus](benchmarks/65816-local-accumulator-forwarding/after/tables.md)
as the integration baseline: 84 native tests per host mode, 302 matching saved
artifacts, and 76 static forwarded sites. Recheck hashes at implementation time;
this design note does not rerun or renew that qualification.

Required evidence for implementation:

- State snapshots and encoded-byte assertions for every modeled instruction
  class. Port 6502 dependency/barrier regressions; include unknown equality,
  stale register aliases, C/V preservation, CMP/LDY changing flags without A,
  partial overwrites, reused stack slots and zero-byte compiler barriers.
- Native mode/width cases: hidden B, narrowing X/Y, byte-to-word transitions,
  status masks, special transfers, unknown status restores and correct fallback.
  Check S movement, malformed/missing homes, DP overlap and banked addresses.
- Independent ca65 original/transformed sequences executed on the qualified VM,
  comparing all relevant CPU state and exact memory traces. Check effects against
  execution, not against a second copy of the same transfer-function table.
- Serialized raw and optimized images, both I states, calls that clobber A/X/Y
  and every scratch byte, aliasing/volatile accesses, recursion, cyclic edges,
  stack-fault raw register state and o65 at both established placements.
- Extend IRQ/NMI coverage to newly live facts; retain the current carry,
  comparison and teardown probes. Keep full-register and frame restoration
  checks rather than checking final results alone.
- Broader forwarding needs a new independent proof index that validates the
  intervening final instructions and effects. The current adjacent-span index
  must keep its strict meaning. Do not simply relax it or use tracker decisions
  as the expected-result/count oracle. Give each new optimization separate
  metrics and exact instruction/cycle/traffic accounting.
- Stage 2 requires identical saved code and behavior in both host builds.
  Later stages require predeclared deltas, unchanged unaffected artifacts and
  retained semantic coverage. Preserve the known external vbcc optimized unlink
  failure explicitly; it is not an expected-success exemption.

Run native tests through the [qualification runner](../tools/native65816-runtime-tests/README.md),
with scoped compiler tests and the [comparison workflow](../tools/compare65816/README.md)
when emission changes. Normalize host fixture newlines and verify affected
LF/CRLF paths. NIR/semantic changes are outside this design; crossing that
boundary requires its separate contributor checks. Update the emission contract
only as behavior is implemented and commit each completed, qualified slice.
