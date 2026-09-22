# Native incoming-parameter word forwarding implementation plan

Status: implemented in `4d7fcb3` and qualified on 2026-09-22. See the
[measured results](MIR65816_PARAMETER_FORWARDING.md). The original plan below
was prepared against main `5972915`, with qualified compiler `55f1061`. Its
[frozen baseline and forecasts](benchmarks/65816-parameter-forwarding/baseline.json)
use the [frame-forwarding results](MIR65816_FRAME_FORWARDING.md).

## Objective and scope

Remove a repeated direct word load of an immutable incoming parameter when A16
and full N/Z still describe that exact word. Support two bounded MIR sequences
within one block:

1. Parameter Load into a private temporary, then Load of the same parameter.
2. Parameter Load into a private temporary, one Store of that captured temporary
   into a disjoint non-addressable frame word, then Load of the same parameter.

Retain the original parameter read, every temporary capture and frame store,
all allocation/home decisions, ABI v1, image v3, o65 profile v1, stack guards,
interrupt reserves and Exec816's compiler pin. Select from verified MIR65816
facts in both frontend modes. No NIR optimization, SemIR lookup, general load
cache, coalescing, X/Y/DP allocation or changed call convention is in scope.

## Rechecked baseline

Planning verified the 425 qualified compiler/fixture input hashes, 474 native
artifacts in each host profile, all 224 comparison artifact hashes and the
identical 264 debug/release records. A fresh read-only typed export recompiles
all 28 Action images and checks complete serialized equality; its 56 LF/CRLF
compilations also match. Output goes to a new ignored file,
`target/parameter-forwarding-plan-facts.json`; historical inventory files stay
unchanged. The two selected raw images remain byte-identical to the earlier
movement inventory's VM-observed images, so its reached load counts still apply.

Only raw `loop_rotation` and raw `recursive_sum` are forecast to change.
All 26 other Action builds, including every optimized build, and all vbcc
artifacts/results should retain their code. These are forecasts, not results
from an implemented optimization.

| Input 13 | Raw rotation, before → forecast | Raw recursion, before → forecast |
| --- | ---: | ---: |
| Code bytes | 172 → 170 | 208 → 206 |
| Cycles | 1,226 → 1,221 | 3,467 → 3,402 |
| Instructions | 304 → 303 | 1,059 → 1,046 |
| Stack-byte reads | 135 → 133 | 256 → 230 |
| Stack-byte writes | 216 → 216 | 290 → 290 |
| Observed stack peak | 18 → 18 | 190 → 190 |

Rotation executes its selected reload once for each of six vectors. Recursion
executes it 0, 1, 8 and 13 times for arguments 0, 1, 8 and 13. Across vectors,
per incoming I state, the ceiling is 28 instructions, 140 cycles and 56
stack-byte reads. Static savings are four bytes across two builds, counted once.
The recursion zero case still loses two static bytes but has no dynamic saving.
Fixed frames remain 18 bytes for rotation and 8 for recursion; the latter's
190-byte peak includes recursive invocations and call transfers.

### Exact selected sequences

Raw rotation, routine 0/block 0, operations 0–2, first argument at S+$16:

```asm
01002E  LDA $16,S
010030  STA $0C,S       ; retained t0 capture
010032  STA $02,S       ; retained store of t0 into local a
010034  LDA $16,S       ; omit only this instruction
010036  STA $0C,S       ; retained t1 capture, even though it reuses t0's slot
```

The Store already consumes the existing temp-forwarding witness: it emits only
STA. Keeping parameter provenance across this particular store is required to
reach the candidate. Do not allow arbitrary intervening stores or extend the
existing temp witness's lifetime. The consumer's logical TempId changes even
when its physical capture slot equals the old one.

Raw recursion, routine 0/block 2, operations 0–1, first argument at S+$0C:

```asm
010056  LDA $0C,S
010058  STA $02,S       ; retained t3, live across the later recursive call
01005A  LDA $0C,S       ; omit only this instruction
01005C  STA $06,S       ; retained t4 capture
```

The new consumer stores move to $010034 and $01005A respectively. Each later
instruction and routine in the affected executable segment moves two bytes.
Calls, guard targets, branches, PER/return fixups and proof positions must follow
finalized code positions. Recursive calls continue to read invocation-owned
stack values; nothing is kept in registers across the call.

## Eligibility from typed target facts

Add a checked incoming-word classifier beside the existing frame classifier in
[accumulator.rs](../src/mir65816/emit/accumulator.rs), using the parameter plan and
final [allocated home helpers](../src/mir65816/emit/copies.rs).

Require a direct, nonindexed `Parameter(ParamId)` address with displacement zero,
a nonvolatile two-byte Load, a two-byte `StackArgument`, and no authoritative
`frame_object`. Resolve the actual incoming displacement using the allocated
frame extent and ABI helper; do not reuse the pre-allocation body offset.
Preflight both bytes, the capture's complete stack extent and zero transient S
displacement before emitting or omitting any instruction. Exclude byte and wider
parameters, interior words of wider arguments, DP captures and unsupported modes.

Normal lowering gives mutable/address-taken parameters an authoritative frame
object. Always reject those, even if an incoming copy happens to retain equal
bits. `verify_routine_plan` validates frame/home layout but does not prove every
operation's memory effects against parameter immutability. Therefore do not use
`frame_object == None` as the sole new proof: conservatively scan typed routine
operations for parameter-address escape or modification. Exclude a parameter
used by AddressOf, as a Store/Copy destination, or through an unapproved access
shape; exclude an inconsistent owned parameter object too. This is a private
optimization classifier, not a NIR pass or new semantic fact table. Hand-mutated
MIR tests must demonstrate refusal despite apparently immutable metadata.

The optional Store must be nonvolatile, direct and two bytes, use the exact
producer TempId/home, and target a non-addressable `AutomaticFrame` word.
Validate object identity, extent and physical disjointness from the incoming
word and capture. Reject external, indirect, indexed, volatile, partial and
possibly aliased writes. An unrelated store of equal bits is not eligible.

## State representation and permitted transitions

The current single `AdjacentWord` holds either `Temp` or `Frame` identity.
Replacing that with a parameter identity would discard existing temp-forwarding
permission immediately after a parameter capture. Keep a separate, narrowly
scoped `IncomingWord` witness in [state.rs](../src/mir65816/emit/state.rs).
It coexists with the ordinary adjacent witness and contains:

- ParamId, exact incoming slot/width and a read-content generation;
- the immutable full-word value identity and its matching N/Z relation;
- producer capture TempId, slot and generation;
- instruction/label cursor and phase: captured, or carried through one Store.

The witness belongs to the active invocation, never just to a lexical ParamId
or numeric stack address. Calls, recursion, joins and S changes clear it.

### Establish only from an actual checked read

Add a typed tracker operation for a checked incoming LDA16. Record the memory/A
relation from the actual LDA effect, then verify it through the retained capture
STA before publishing the witness. The read fact can use the existing exact-home
value/generation machinery, with a separate read admission rule. Do not register
incoming arguments as ordinary writable private homes, manufacture an incoming
store, or seed a read fact merely because A currently contains a plausible value.
Do not make generic `read_stack` a cache for all stack memory.

Preserve the existing temporary publication after the capture. The incoming
witness must also survive that publication, without weakening the public meaning
of `barrier()`: a general operation barrier must still clear both permissions.
Record any new observable home relation at an appropriate trace boundary so VM
trace checks can verify it, rather than silently trusting selector metadata.

### Carry through at most one retained Store

The optional Store transition is an explicit typed permission. Before selection,
require the current incoming/capture facts and the matching existing temp
witness. Complete the Store's existing preflight. It may carry the incoming
witness only if its source LDA is actually omitted and the only emitted
instruction is the admitted A16 STA to the checked disjoint object word.

After that STA, recheck the source read generation, capture generation, A/N/Z,
zero stack delta and unchanged label cursor, advancing the instruction cursor
by exactly that store. Retain normal frame-word witness publication. Do not
restore a saved witness across a generic barrier merely because A still matches;
the dedicated transition must account for the entire instruction effect.
A second Store is outside this slice and revokes incoming permission.

This is the only allowed intervening operation. NOP, CLC, LDY, CMP, arbitrary
stores, helpers, calls, pointer formation, stack operations and mode transitions
break the proof even when some register bits happen to remain equal. An A16
request that emits no instruction does not itself break adjacency.

### Consume once at the matching Load

Check typed eligibility and every source/destination extent before consumption.
Require the same ParamId and exact incoming word, matching generation, A16 and
full N/Z, zero transient S and the expected instruction/label cursor. On success,
omit only LDA and emit the original capture STA. Publish its ordinary temp
witness so the next arithmetic/comparison/store/return retains current behavior.

Do not rearm incoming permission from an omitted read in this first slice. A new
actual incoming read may establish a fresh witness. On an unsupported or failed
proof, use the existing load path and retain its diagnostics. Calls and joins
never carry either permission; source-memory ordering and all stores remain.

## Independent evidence and regression coverage

Add a separate test-only parameter-forwarding index under native test support.
Derive it from verified MIR, exact parameter/capture homes and operation spans;
independently decode final instructions and check the full allowed window.
Recognize both sequences, including the optional typed Store. Reject alternate
label entries, unexpected instructions, operand-byte mutations and mismatched
identities. Rebase every proof address for o65 placements. A compiler witness
must not serve as its own qualification evidence.

Extend the existing temp index only to recognize a parameter load whose machine
span is now its retained STA; obtain its A/N/Z evidence from the independently
checked parameter window, as frame forwarding already does. Keep existing
`forwarded_word_loads` and `frame_forwarded_loads` meanings and counts unchanged.
Add Action-only `parameter_forwarded_loads` and `parameter_forwarded_load_sites`;
vbcc's record schema remains unchanged. Count reached CPU instruction boundaries,
checking A against the original incoming bytes and full N/Z each time.

Coverage must include:

- Both sequences, distinct and reused capture slots, arbitrary incoming C/V,
  zero/sign/wrap boundaries, raw and optimized frontend modes, and parameters
  other than argument zero. If optimized source removes a positive sequence,
  supplement source fixtures with verified typed MIR probes rather than weakening
  assertions or introducing a special case for a corpus program.
- One optional Store accepted, two or unrelated stores rejected; a third Load
  after an omitted consumer must perform an actual read before rearming.
- Different ParamIds with equal bits, incorrect homes/generations, partial
  overlaps, stale flags, width changes, labels, zero-byte MIR barriers, calls,
  helper and indirect clobbers, address-taking and mutable authoritative homes.
- Last-byte d,S boundaries, zero/255/overflowing offsets, final-frame incoming
  displacement, transient outgoing arguments, invalid capture extent and refusal
  without partially emitted optimized code.
- Recursive calls and two task domains sharing the lexical parameter but using
  different live argument values. Inject IRQ and NMI at the initial read, each
  retained store and consumer boundary. Compare full registers and live argument,
  frame and return memory with independent uninterrupted execution. Include
  seeded schedules and both incoming I masks where applicable.
- Two o65 placements, actual serialized images, trace-on/off byte and metadata
  equality, and VM checks of simultaneous A/home/NZ generations. Normalize host
  text and rebuild changed fixture consumers in an isolated CRLF checkout.

Add an exact corpus checker using the frozen old images, not optimized output
to infer the expected transformation. Delete only the two specified LDA
instructions and remap all position-bearing content, including uncounted
routines, guard/call records, branch/PER references and instruction/site counters.
Verify complete images and all 264 records. A target into a removed load is a
rejection, not something to redirect silently. Require unchanged ABI, frames,
staging, stores, guard counts/costs, DP traffic, results and existing forwarding
counts, plus the frozen per-vector savings including zero execution.

The reviewed emission snapshot includes raw `recursive_sum` and therefore needs
an intentional machine-code/proof-offset update. Derive the expected changes
from its frozen bytes and metadata before refreshing it. Other snapshot sections
must remain unchanged unless separately explained and qualified. No NIR fixture
or printer contract change is expected.

## Delivery and acceptance

1. Commit this plan and its checked baseline/forecasts. Keep current historical
   qualification, movement inventory and saved compiler artifacts immutable.
2. Implement the read witness, both bounded selectors and independent evidence
   together. Add focused state/selector and native tests, including refusals and
   interrupt restoration. Update the emission contract and reviewed machine
   snapshot. Commit the completed behavioral slice after relevant checks pass.
3. Build and measure a fresh `target/parameter-forwarding-after` corpus. Run both
   host profiles with paired I states, exact image/record checking, full native
   qualification and CRLF validation. Commit portable results, qualification and
   the quality-plan baseline update. Leave coalescing and scalar DP for separate
   measured slices.

Use the affected checks:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --lib mir65816 --features native65816-state-proof
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --test mir65816_state_boundary --test mir65816_abi \
  --test mir65816_contract --test mir65816_emission --test mir65816_o65 \
  --test actionc_65816_cli --test actionc_65816_o65_cli
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py
CARGO_INCREMENTAL=0 python3 -B tools/native65816-runtime-tests/qualify.py --release
python3 -B -m unittest discover -s tools/compare65816 -p 'test_*.py'
python3 -B tools/compare65816/corpus.py --check
```

Build the release compiler and run the comparison builder with `--verify-crlf`.
Run the ignored `code_quality` target with `A816_COMPARISON_MANIFEST` pointing to
the new corpus and distinct debug/release results files. The known optimized
vbcc `unlink` vector-0 failure must remain reported; no new failure is accepted.
Run the new exact checker against `target/frame-forwarding-after`, then snapshot
with `report.py`. Finish comparison-tool edits before the final build so recorded
input hashes match. Retain all unrelated worktree changes and stage explicit paths.

The full root suite and NIR sweep are unnecessary if implementation stays within
this target boundary. Broaden validation if an actual contract change or failure
requires it. Acceptance is correct emitted code with the declared reductions,
not merely a passing pattern test or an apparent A-register equality.
