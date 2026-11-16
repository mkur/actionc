# MIR6502 call input and result contracts

Status, 2026-11-17: the input/state contract for issue **3** and the
[typed result contract](#typed-direct-result-contract) for issue **4** are
implemented. Covers the corresponding entries in the
[fact repair priorities](MIR6502_FACT_REPAIR_PRIORITIES.md), based on the
[fact-contract audit](MIR6502_FACT_CONTRACT_AUDIT_2026-11-16.md).

A call must supply its ABI arguments and satisfy any genuine extra machine
requirements. Reading existing program state does not create another argument.
Keep those obligations separate from the memory reads that constrain storage
placement, deletion and reordering.

This distinction belongs in MIR6502. Existing typed storage identities, ABI
descriptions and machine operands provide the evidence; consumers must not look
back into SemIR or infer a role from a source name. Flag-definition coverage and
physical-overlap precision remain the separate repairs numbered 5–6.

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
