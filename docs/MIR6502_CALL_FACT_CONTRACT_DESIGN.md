# MIR6502 call and machine fact contracts

Status, 2026-11-17: the input/state contract for issue **3** and the
[typed result contract](#typed-direct-result-contract) for issue **4** are
implemented. The [guaranteed register/flag contract](#guaranteed-register-and-flag-definitions)
for issue **5** is proposed. Covers the corresponding entries in the
[fact repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md), based on the
[fact-contract audit](MIR6502_FACT_CONTRACT_AUDIT_2026-11-16.md).

A call must supply its ABI arguments and satisfy any genuine extra machine
requirements. Reading existing program state does not create another argument.
Keep those obligations separate from the memory reads that constrain storage
placement, deletion and reordering.

This distinction belongs in MIR6502. Existing typed storage identities, ABI
descriptions and machine operands provide the evidence; consumers must not look
back into SemIR or infer a role from a source name. The final section covers
register/flag definitions; physical-overlap precision remains issue 6.

## The defect

TN's `ArenaCursor()` reads the global pointer `allocp` at `$E8/$E9` without
taking arguments. The audited scalar entry analysis projected those reads into
`Inputs.fixed`. Handoff and residence then required every member of that set to
appear in the outgoing argument lanes. The global read became a caller setup
obligation.

The audit's independent `ReadState()` control demonstrates the address dependence:
a global at `$E8` produces `extra-entry-fixed-byte`, while the same global at
`$0600` does not. Moving existing state must not change the routine's argument
contract. It may change instruction selection and physical interference.

## Three separate contracts

| Contract | Meaning | Existing authority |
| --- | --- | --- |
| Canonical arguments | Values this invocation must pass in the declared ABI lanes, including unused formal arguments | `direct_parameter_abi`, call arguments and `abi::canonical_call_lanes` |
| Machine entry requirements | Incoming registers, flags, stack state or physical home bytes observed before a guaranteed definition | `MirCalleeEntryFact` and its native or bounded-machine entry analysis |
| Program memory observations | Memory the invocation may read, including globals, explicit addresses and pointees | `MirKnownMemoryReads`, structured effects and explicit machine storage observations |

The second contract includes observed canonical lanes and any additional
implicit inputs. It need not duplicate the formal parameter list: comparing it
with the canonical lanes identifies the extras. The third contract remains
independent even when some of its physical bytes coincide with argument homes.

Use the existing memory summaries for program reads. Do not add a second
transitive global-state summary or a new global-initialization analysis to the
entry fact. A global's value can have been established much earlier, by another
routine, or by the environment. Its validity is not proved by the immediate
caller's argument setup.

`MirEntryRequirements` retains the register, individual-flag and fixed-byte
fields formerly grouped under `Inputs`. Its contract is machine
entry dependence, not all memory dependence. An unavailable entry fact remains
unavailable; an empty requirement set is a successful proof, not a fallback.

## Preserve the origin of each read

Classify an access before reducing it to a physical ZP byte. The initial positive
case is a typed source-global access, including a valid global alias and byte
offset. It is a program-state observation regardless of whether its backing is
ordinary RAM, absolute RAM or ZP. Both bytes of a global pointer are state reads.

| Access evidence | Treatment in entry analysis |
| --- | --- |
| Typed `MirMem::Global` byte or a typed assembly relocation to that global | Keep the program read; do not generate a caller-supplied home requirement |
| Parameter capture, retained ABI lane, bound parameter/local home, or compiler pointer scratch | Keep the existing machine-home dependency and definition analysis |
| Literal fixed-ZP access without evidence of a program-state role | Retain the physical entry requirement when read before definition |
| Ordinary indirect data read | Keep its memory footprint, possibly Unknown; analyze the pointer carrier and index inputs separately |
| Unsupported or lost operand evidence needed for an entry proof | Retain a conservative physical requirement where representable, otherwise decline that proof |

This repair does not reclassify all locals or parameter homes as persistent
state. Their capture, initialization and residence eligibility retain their
existing proofs. It also grants no new permission to delete or reorder global,
absolute, hardware or indirect reads.

The classification is per access. Never subtract all declared global addresses
from a completed fixed-input set. A typed global read and an independent raw
scratch read can name the same byte; the latter still contributes a requirement.
Likewise, naming a global at `$A0` does not prove an ABI result definition, and
naming one at `$AC` does not certify a raw assembly pointer input.

A literal assembly read at `$E8` is therefore conservative even if a global
happens to occupy `$E8`. A typed reference to that global provides the missing
role evidence. No address whitelist or TN-specific rule is permitted.

## Native transfer and assembly evidence

In [native entry analysis](../src/mir6502/analysis/known_callees/entry_inputs.rs),
classify direct read ranges before the current `physical` projection. A proven
global read stays in ordinary effect and observation summaries but does not enter
the home transfer used to compute machine entry requirements. Reads of genuine
ABI or scratch homes still participate in that transfer.

Keep the rest of the operation's effects. Loading a global into A still defines
A and the appropriate flags. An explicit store to a global with proven fixed
backing can still establish a physical byte subsequently consumed as an implicit
input. That requires exact address evidence and a guaranteed write on every
relevant path. Unknown placement cannot discharge a physical requirement.
Possible writes cannot discharge one either; use the distinction established by
issue 2 in the [pseudo-machine contract](MIR6502_PSEUDO_MACHINE_CONTRACT.md).

For [bounded assembly entries](../src/mir6502/analysis/known_callees/bounded_memory/entry_inputs.rs),
retain typed operand identity through decoding. Reuse the typed relocation and
`StorageAddress` evidence already handled by
[machine storage observations](../src/mir6502/analysis/known_callees/observations.rs).
Resolving a global relocation to `$E8` must not erase the fact that the operand
refers to that global. Any retained association belongs to the decoded operand
and the exact machine body, not to an address-only lookup table.

An indirect instruction has separate pointer-cell and pointee observations.
A typed global pointer cell is existing state; a raw incoming `$AC/$AD` pair
remains an implicit machine input unless the body establishes it. The pointee
read can remain Unknown without turning every possible pointee byte into an
argument. A may-read or may-write annotation alone establishes neither an
initial value nor an extra ABI convention.

Preserve the current bounded instruction and control-flow coverage. This change
does not add assembly leaves to acyclic residence admission; that remains issue
9. Missing evidence declines an optimization proof, not the source program.

## Composition and call admission

At every call site:

1. Validate outgoing byte count and canonical physical lane order against the
   declared direct ABI through `canonical_call_lanes`, including repeated calls
   to the same target.
2. Check the callee's machine entry requirements against those lanes and any
   already-supported explicit supply proof. An extra register, flag, stack or
   scratch requirement still blocks the optimization when unsatisfied.
3. Apply the independent read-observation, possible-write and preservation
   checks required by the proposed home change.

In [acyclic composition](../src/mir6502/analysis/known_callees/entry_inputs/acyclic.rs),
only canonical lanes and machine entry requirements feed the incoming-machine
dataflow. Propagate them through actual setup and the CFG using guaranteed
definitions. Program-state reads compose through the existing memory summaries
and never become new argument lanes in an ancestor.

For example, `Outer()` calling `ReadState()` inherits its observation of `state`,
not a requirement to supply `$E8`. If the child instead consumes an undeclared
incoming scratch byte, that requirement propagates unless `Outer` establishes
it on every path reaching the call. A conditional store or callee may-write is
insufficient. All formal argument lanes remain mandatory in either case.

The existing `PlannedCapture` and `ActualEntry` stages retain their meaning.
Planned capture may discharge working-home dependencies only through its
certified synthetic capture. Actual entry uses the expanded body. Ordinary
dead-call fallback eligibility in residence is unchanged and must not become a
complete entry certificate for another consumer.

## Storage safety and validity

Removing a false setup obligation does not release the underlying bytes. Source
globals keep their existing ZP reservations, alias observations and storage
ownership. Reads of `allocp` still constrain deletion and movement of stores to
`allocp`; its writes still constrain caller values that survive the invocation.
Global writes must not become apparent ZP preservation merely because global
reads stopped appearing in an argument check.

Use logical identity to determine the read's role and the existing authoritative
backing or layout to determine physical overlap. Do not infer state ownership
from whichever object happens to occupy an address after a trial layout. No
physical-overlap precision beyond the existing resolver is required for issue 3.

Entry facts remain scoped to their source body, ABI, entry stage, bindings,
machine payloads and relevant global backing/alias information. A change to any
evidence used for classification invalidates the fact, including a change in a
descendant. Reuse the current owned snapshots and transitive freshness checks;
extend their dependencies if the classifier consumes additional metadata. Do
not introduce a persistent cross-candidate cache.

## Producers and consumers

| Component | Required change or invariant |
| --- | --- |
| Native and bounded-machine entry producers | Preserve access role before fixed-byte projection; report only machine requirements |
| Acyclic entry composition | Carry machine requirements through setup; leave state reads in memory summaries |
| `handoff::inputs_supplied` and `ResidenceCalls::inputs` | Consume the clarified requirements and retain per-call canonical validation |
| Known memory summaries and machine observations | Continue reporting the same state accesses and conservative uncertainty |
| ZP ownership, reservation and preservation checks | Continue protecting source state and genuine implicit inputs |
| Entry diagnostics and traces | Distinguish formal lanes, extra machine requirements and memory reads; do not label a global read as a missing argument |

## Focused acceptance cases

- A no-argument global reader at `$E8` and the same reader at `$0600` have the
  same caller-supply contract. Their read footprints still identify the global;
  the ZP version still reserves its byte.
- Repeat with a two-byte global, a valid alias plus offset, and an acyclic
  wrapper. State observations survive composition without extra fixed inputs.
- A routine that both reads global state and consumes real argument lanes must
  still reject missing, reordered or incomplete arguments.
- A raw assembly scratch read remains an extra requirement. Adding a coincident
  global declaration must not suppress it. A typed global relocation is the
  positive control; mixed typed and raw reads retain the raw requirement.
- A definite store can satisfy an implicit byte requirement on all paths;
  a conditional store or finite may-write cannot. Missing result typing and
  incoming-flag dependencies keep their current conservative behavior.
- Changed backing, alias, machine operand or descendant invalidates cached
  entry facts. Planned and actual stages cannot reuse each other's facts.
- One small VM control reads global state through a call while a caller value
  stays live. Exercise both runtimes and verify the observed global value and
  caller result; keep the unit cases responsible for the classification matrix.

For TN, report that `allocp` remains a two-byte program read and is absent from
caller-supply requirements. `ArenaCursor` may still encounter the independent
flag and physical-write blockers from the audit. ZP placement or binary-size
improvement is not an acceptance condition for this contract repair.

## Typed direct result contract

Implemented for issue **4**. A routine's result ABI must remain known when its
parameters include pointers. Result typing identifies the permitted output
homes; a separate body proof establishes which bytes are actually defined on
every returning path.

`MirScalarSignature` combines integer-only parameter and result
facts. A pointer parameter makes that entire signature unavailable.
`direct_parameter_abi` supports pointer parameters but does not retain a result
contract. Previously, `acyclic::defined_results` therefore could not certify
the ordinary return slots.
In the TN qualification after issue 3, `Next` requires A/X and writes `$A0/$A1`,
but `Ord` still inherits `$A0` as an input alongside its genuine `$A3` argument.
`Items` also inherits `$A0`. A possible-write footprint cannot repair this gap.

### Representation and ownership

Add `MirRoutine.direct_result_abi: Option<MirDirectResultAbi>`. This describes
the routine's declared direct result independently of its parameter types and
storage frame. Populate it during MIR6502 lowering from the verified NIR
callable result type, without consulting SemIR or emitted map metadata.

| Contract value | Meaning under the supported Action ABI |
| --- | --- |
| `None` | Unknown or unsupported result contract; publish no guaranteed result homes |
| `Some(Void)` | Known procedure with no result homes |
| `Some(Scalar(Byte))` | Declared byte result at `$A0` |
| `Some(Scalar(Word))` | Declared word result at `$A0/$A1` |
| `Some(Pointer)` | Declared 16-bit data-pointer result at `$A0/$A1` |

`MirDirectResultAbi` has the `Void`, `Scalar(MirWidth)` and `Pointer` variants.
Keep Void distinct from Unknown. Accept the supported byte/word integer types
and 16-bit pointers in the data address space. LONG, REAL, aggregates, other
address spaces and untyped generated entries remain unsupported by this narrow
contract. Their existing lowering and execution behavior is unaffected.

Determine result metadata independently even when the parameter ABI is
unsupported or variadic. That does not admit such a routine to a proof: current
entry, direct-parameter and call-closure restrictions still apply. Retain
`MirScalarSignature` for its existing integer-specialization consumers; do not
broaden its meaning or use it as a fallback for missing direct-result metadata.

Use one MIR6502 ABI helper to map the declared direct result to its canonical
byte homes, consistent with existing Action return lowering. The contract adds
no A/X return convention and does not change call setup or emitted returns.
Explicit runtime-helper result homes retain their existing contracts.

### Guaranteed definitions on return

Reuse the existing CFG and guaranteed home transfers in
`acyclic::defined_results`, changing the source of the candidate result bytes.
Share the proof between supported native `ActualEntry` leaf facts and acyclic
facts so querying the same leaf through either path gives the same answer.
`PlannedCapture` remains an input proof and does not certify expanded outputs.

For each declared result byte, ask whether every reachable path to a normal
return contains a guaranteed definition of that byte. There must be at least
one reachable normal return. Ignore unreachable returns; loops and joins retain
the existing conservative CFG treatment. A call can supply a definition only
through a current, certified result fact for its target.

Publish the proven subset of the declared bytes. A low-byte store alone may
prove `$A0`, but cannot also prove `$A1`. Conditional stores, possible clobbers,
write annotations and a signature alone cannot supply missing definitions.
Incidental stores to other ABI or scratch bytes do not become exported results.
A void or unknown result contract exports no result bytes even if the body
happens to write `$A0`.

This fact says that a normal return has a callee definition for the byte. It
does not describe its value or independence from call inputs, prove that memory
is unchanged, certify a pointer range, or establish register/flag preservation.

### Composition and validity

At a supported call, observe canonical arguments and genuine entry requirements
before applying the certified result definitions. A callee may both consume
incoming `$A0` and define a result there; its result must not hide that input.
Apply definitions only on the normal-return continuation and only to the
certified bytes. The existing possible-write and memory-read footprints remain
independent, including Unknown pointee reads.

The result declaration must agree with the supported Action ABI before its homes
can be certified. When validating a call that consumes a result, a conflicting
known result width or home cannot be justified from an incidental store or a
caller annotation; reject the proof. Ignoring a declared result remains valid.
No new assembly-call composition, recursive proof or exceptional-return model
is introduced by this repair. The result proof remains limited to supported
native bodies; a typed declaration alone never certifies a machine block.

Carry the result contract through routine cloning and materialization. Generated
routines without an authoritative typed interface use Unknown. Hand-built test
routines must declare the result contract explicitly when they expect an output
proof. Print the contract in MIR diagnostics so Unknown and Void are visible.

Owned routine snapshots must include this metadata. Changing only a child's
result declaration, body or ABI invalidates its fact and every dependent caller
fact, even when the caller's instructions do not change. Recompute from the
candidate program before accepting home changes; do not retrofit an old summary
with a new result width.

### Focused acceptance cases for results

- Identical byte-returning bodies with CARD and pointer parameters publish the
  same `$A0` definition. Include a caller that explicitly reads `$A0` after the
  call, so the test exercises composition rather than only metadata lowering.
- Byte, word, data-pointer, Void and Unknown results remain distinct. Changing
  parameter types alone does not alter supported result metadata. Unsupported
  entry or parameter ABIs remain outside the existing proof scope.
- Both word bytes defined on every returning path are certified; a missing or
  conditional high-byte definition does not erase an incoming `$A1` dependency.
  A may-write cannot substitute for either byte. A body with no reachable normal
  return publishes no result guarantee.
- An incoming result-slot read remains an entry requirement even if that slot
  is later returned. Scratch writes and void routines produce no extra outputs.
- Direct leaf and acyclic queries agree. Mutating only a descendant's result
  contract invalidates the cached outer fact.
- One focused VM control checks a caller value retained across pointer-parameter
  calls with byte/word results, including carry and page boundaries, in both
  runtimes. Reuse the independent fact controls for the rest of the matrix.

For TN, the expected fact correction is that `Next` can certify `$A0/$A1` and
its callers no longer inherit `$A0` solely from those result reads. `Ord` must
retain its real `$A3` argument requirement. Other entry, write-footprint and
placement blockers remain independently authoritative; size improvement is a
measurement, not the proof's acceptance condition.

## Guaranteed register and flag definitions

Implemented and qualified for issue **5**, 2026-11-17. Only guaranteed definitions may kill
incoming register or flag values in liveness. A possible clobber invalidates
value knowledge but cannot establish a definition. Apply this distinction to
A/X/Y and individual C/Z/N/V flags as consistently as issue 2 applies it to homes.

### Motivating disagreement

Before this repair, the [effect classifier](../src/mir6502/analysis/effects.rs)
reported broad machine writes and clobbers. Ordinary [machine liveness](../src/mir6502/analysis/machine_liveness.rs)
merged `register_writes` with `register_clobbers`, and `flag_writes` with
`flag_clobbers`. Entry analysis then replaced only the flag definitions with a
separate `entry_flag_transfer` whitelist. Register definitions still inherited the
ordinary transfer. Neither path provided a consistent guaranteed-definition rule.

There were also two flag-entry answers: `scalar_entry_ignores_flags` uses ordinary
liveness on a cloned body with conservative call observations, while
`MirKnownCalleeEntryFlags` uses the narrower entry transfer. The shared native
entry fact uses the latter transfer, with optional certified call inputs.

For the selected byte operation that emits `CMP #$28` followed by `BCS`, C/N/Z
are established by the comparison. The entry whitelist omits this ordinary
immediate form and retains incoming C. This is the false dependency reported for
the independent comparison controls and TN's `ArenaCursor`.

Simply accepting every `Compare` is also wrong. In
[comparison emission](../src/mir6502/emit.rs), Boolean compares can fold to
constant loads, and equality with zero can call `emit_cmp_imm_for_z_branch`.
The [tracked emitter](../src/codegen/tracked_emitter.rs) may omit that CMP when
the existing Z/N flags already describe A. Such a comparison does not guarantee
a fresh C definition. The old `definitely_overwrites_carry` assignment for
all flag-destination compares is therefore broader than its name promises.

### One authority, using the existing effect structure

Make the existing `MirMachineEffects.register_writes` and `flag_writes` the
guaranteed machine-definition sets. Audit their producers against the supported
MIR recipes and emission. Move merely possible changes into the existing
clobber sets. Keep `register_clobbers`, `conservative_register_clobbers` and
`flag_clobbers` available for invalidation and preservation; none is a liveness
kill. Do not add another parallel opcode summary or persistent fact cache.

The contract for a guaranteed write is: every valid execution of this operation
that reaches its continuation establishes the corresponding machine value. This
is a boundary guarantee, not a demand for a fresh machine instruction when a
proven equivalent value is already available. Its value may depend on an input:
`ADC` with `FromPrevious` both reads and defines
C. That input read must survive. A definition is not a preservation claim or a
claim that the resulting value is independent of the inputs.

For each operation or terminator, the shared transfer is:

```text
live_before = reads union (live_after minus guaranteed_writes)
```

Reads precede writes within the operation. CFG joins retain a requirement if any
reachable path needs the incoming value. An unsupported recipe contributes no
guaranteed physical definition; retain its possible effects and conservative
reads. Uncertainty must not turn into a successful empty entry fact.

Keep logical temp definitions separate. A pre-materialization `Compare` can
define a Boolean temp without promising any particular physical flag recipe.
Do not derive physical guarantees from its result type, destination name, or
the union of registers touched by all possible implementations.

### Selected forms and emission

Start with the currently certified selected forms, moved into the central
classifier, and add the missing unsigned byte comparisons after checking their
actual emission. The rules must account for width, operands, destination,
operator and carry mode; an opcode-family name alone is insufficient.

| Selected behavior | Guaranteed-definition rule |
| --- | --- |
| Unconditional byte load or non-identity register transfer | Establish the destination and the flags the actual recipe defines |
| A self-move or zero-count shift | No fresh physical definition from the omitted operation; retain genuine operand reads |
| Unsigned byte relational comparison using A/X/Y and an immediate | CMP/CPX/CPY establish C/N/Z; V remains unchanged |
| Equality comparison whose zero CMP may be omitted | No C definition; claim Z/N only if the emitted and reused paths both establish their required postconditions |
| ALU operation with an internal carry initializer | Its incoming C is not read; use the recipe's guaranteed output flags |
| ALU operation with `FromPrevious` | Retain the C read even when the same operation later defines C |
| Variable-count or internally conditional recipe | Intersect definitions across all paths, including the zero-work path |
| Possible clobber annotation | No guaranteed definition |

An elided instruction may be safe because an earlier instruction established
the needed value or flag. Audit the whole selected recipe, including elision:
the same claimed postcondition must hold on every path, with genuine input reads
retained. For example, tracked LDA/LDY load elision checks that the corresponding
Z/N values already match, while omitted `CMP #0` does not establish carry. If
this guarantee cannot be proved, leave that definition out; earlier operations
can still discharge dependencies through normal dataflow. Do not add a second
state-tracker replay in this repair.

Keep existing emission and flag-reuse optimizations. This work repairs the facts
describing their behavior; it does not force extra CMP instructions merely to
make a summary true. Unsupported or ambiguous forms retain conservative facts.

### Calls, assembly and query scope

Call ABI clobbers and manual assembly clobbers describe possible changes.
They do not initialize registers or flags. The result-home guarantees established
by issue 4 remain separate: defining `$A0/$A1` does not prove A/X or flag outputs.
Only an already-supported explicit physical result recipe may establish such
register outputs. Do not add callee flag-result summaries in this repair.

An uncertified call retains the existing conservative entry-observation policy.
A certified acyclic call uses the child's actual input requirements. Neither
policy turns possible call clobbers into definitions. Preserve canonical argument
validation, reads-before-results ordering, and the distinction between explicit
stack dependence and the ordinary balanced call/return convention.

The bounded assembly decoder already derives instruction-level reads and writes
from actual opcodes. Keep it as the authority for supported decoded instructions;
annotations alone cannot replace that evidence. It should agree with native
selected forms when both describe the same instructions. This does not expand
decoded opcode coverage or admit assembly leaves to acyclic residence; issue 9
remains separate.

Queries may retain different documented eligibility and call policies. The
integer-only flag wrapper and broader native-entry analysis need not accept
exactly the same routines. On the same admitted body, entry stage and call policy,
their flag answers must agree. Define `scalar_entry_ignores_flags` as the
all-flags-unobserved projection of the shared flag-entry result; remove its
independent cloned-body calculation.

### Consumer migration and validity

- Ordinary machine liveness and entry liveness subtract only the central
  guaranteed sets. Remove the local `entry_flag_transfer` definition whitelist;
  keep boundary read policy separate from operation definitions.
- Make `MirKnownCalleeEntryFlags` and `MirCalleeEntryFact` consume that same
  transfer. Continue propagating individual flags through the existing CFG solver.
- Replace the separately assigned `definitely_overwrites_carry` and
  `definitely_overwrites_overflow` facts with projections of guaranteed writes.
  Update `materialize/flags.rs` consumers accordingly. A compatibility predicate
  meaning “may change flags” must instead use the possible-write union.
- Audit remaining uses of the machine write/clobber fields. Value invalidation,
  interference, preservation and rewrite exit-state declarations must continue
  considering all possible changes. Narrowing the definite sets must not
  accidentally claim preservation.

The guarantees belong to the current MIR operation and its supported emission
contract. Width expansion, compare folding, flag reuse or replacement with a
different recipe requires recomputing them. Existing owned body snapshots and
candidate-local analyses remain the validity mechanism. No new cross-candidate
cache or general generation framework is needed here; the broader freshness
audit remains issue 8.

### Focused acceptance and migration order

First establish the central guaranteed-write contract and tests for selected
forms. Then migrate the entry and ordinary-liveness consumers together with the
carry/overflow compatibility queries. Finish with independent runtime controls
and TN qualification. Do not leave the legacy and precise entry answers using
different transfer definitions as the final state.

Required controls:

- Immediate unsigned comparisons remove false C inputs; a later V observation
  still requires incoming V. Cover A/X/Y operands and expanded word comparisons.
- Zero equality with reused Z, self-moves, zero shifts, constant-folded compares
  and conditional recipes cannot manufacture missing flag definitions.
- `ADC`/`SBC` with incoming carry retain that dependency; explicit initialization
  removes it. A definition on only one branch does not discharge a joined input.
- Finite call or machine clobbers followed by register/flag reads retain incoming
  dependencies; possible writes still invalidate tracked values and preservation.
- For matching scope, legacy all-flags queries, precise flag queries, native
  entry facts and ordinary machine liveness agree. Mutating a producer invalidates
  its entry witness and dependent callers.
- Small emitted/VM controls vary initial flags and data at comparison boundaries.
  Test both an emitted CMP and a reused-zero-flag path; preserve later carry use.

For TN, re-audit `ArenaCursor` and callers such as `Alloc`, `Free`, `Push` and
`Pull`. Remove a carry requirement only when every path proves it unnecessary.
Keep independent memory, physical-overlap, entry and placement blockers visible.
Record actual size changes under the existing final-cost and rollback rules;
truthful facts are the acceptance condition, not a promised size reduction.

Issue-5 qualification: 373 focused unit tests, seven loop integration tests,
three callee-fact VM controls and the C/V observer VM matrix passed. The retained
audit covers nine controls and all 95 TN routines. `ArenaCursor` and `Alloc`
now require no incoming flags under certified acyclic composition; the
conservative nested-call policy remains intentionally broader. Issue-4 result
facts remain intact. TN is unchanged at 9,811 bytes, including its image hash.
Two old size-delta integration assertions fail identically on pre-repair
`2ce87047`; their expectations were not changed. See the
[implementation plan](MIR6502_MACHINE_DEFINITIONS_IMPLEMENTATION_PLAN.md) and
[qualification receipt](benchmarks/6502-call-facts-audit/machine-definitions-qualification.json).
