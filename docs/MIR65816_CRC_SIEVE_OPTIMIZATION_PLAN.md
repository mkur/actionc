# MIR65816 CRC and sieve improvement plan

Status: implementation in progress, starting from comparison commit `279a791f`.
Each slice is committed separately with focused tests and measurements in the
[series report](benchmarks/65816-crc-sieve-optimization/README.md).

## Objective and evidence

Improve native 65816 execution speed through small instruction-selection and
private-copy changes that also reduce code size. Preserve the Exec release
budget of 256 KiB for loaded code plus initialized data, excluding debug stack
guards. Do not introduce a new optimization mode or a larger register allocator.

The frozen [CRC comparison](benchmarks/65816-crc-speed/README.md) and
[sieve comparison](benchmarks/65816-sieve-speed/README.md) are the baselines.
Their measured compiler revisions are `7936f5d6` and `73e2a23e`; compiler sources
are unchanged between them. Preserve these reports and their fixture inputs.

| Opportunity | Measured baseline cost | Interpretation |
| --- | ---: | --- |
| CRC32 identity copies in the true arm and loop backedge | 6,303,296 cycles, 26.17% | Largest clearly identified redundant-copy cohort; any required terminal-state repair remains |
| CRC32 false-arm four-byte transfer through staging | 2,085,312 cycles, 8.66% | A real transfer; optimize width/staging, never delete it as an identity |
| Sieve repeated static-array pointer setup | 659,001 cycles, 19.38% | Candidate for long indexed addressing; replacement instructions must be counted |
| Sieve adjacent length captures before comparisons | 332,820 cycles, 9.79% | Candidate for direct comparison with the immutable incoming home |
| Bit sieve helper shifts by three | 1,072,099 cycles of local estimated saving | 46,613 shifts, 39 to 16 cycles each, before surrounding captures |
| CRC32 materialized sign-mask test | 2,752,142 cycles, 11.42% | Cost of the old sequence, not entirely removable work |

CRC figures use the 8,192-byte input. Standard sieve uses 8,191 slots; bit-sieve
figures use its original 16,000 slots. Percentages from different kernels are
not additive, and later slices must be measured against their immediate parent.
These observations do not establish an Exec-wide speedup or byte saving.

Calypsi controls remain CRC8 `-O1 --speed`, CRC16/32 `-O2 --speed`, and both
sieve `-O2 --speed` configurations, with/without `--no-cross-call`. CRC8 O2's ten
known wrong-result cases stay explicitly recorded and outside valid rankings.

## Implementation boundaries

- Work after verified, typed MIR65816, using allocated homes and structured
  operands. No executable source-string matching, SemIR lookback or new NIR
  optimization pass is needed.
- Keep the existing ABI, guards, frame reservations, DP ownership and helper
  interfaces. Initially leave unused captures and edge-staging slots allocated;
  reclaiming their frame storage is a separate future decision.
- Plan and validate an entire replacement before emitting or suppressing any
  instruction, fixup, definition or source span. Unsupported cases retain the
  current path; malformed inputs still fail validation.
- Preserve source-memory access widths, order and multiplicity. All omitted or
  widened transfers in this plan concern proven private homes. Do not adopt
  Calypsi's widened reads of ordinary BYTE arrays as an Action memory contract.
- Calls, machine blocks and unknown memory effects retain existing barriers.
  No cross-call or cross-block register residency is introduced.
- Account for required A/X/Y and flag effects through the tracked emitter.
  Never keep stale value/flag facts after a replacement. New instruction forms
  require matching encoding, effects, state/replay and relocation support.
- Prefer forms that are no larger and faster for the selected case, including
  mode changes and repairs. Do not fund a benchmark win with unexplained Exec
  image growth.

## Ordered commit slices

### 1. Eliminate identity copies from mixed-width edges

Current word-only and pointer copy planners already have specialized handling.
The generic fallback in `select.rs::edge_transfer` stages every byte of every
argument, including a four-byte value already in its destination home.

Add a checked plan for identifying exact source/destination identities in that
fallback. Start with complete private stack homes and exact byte-range equality.
Check the whole edge, not just one move: another destination must not overwrite
any byte of a supposedly preserved identity. Retain full staging for every
remaining move, in its original slot, with all source captures completed before
any destination assignment. Do not add general mixed-width cycle scheduling.

Preserve the required edge-exit state. Where the existing final-copy contract
requires an A/N/Z repair, retain an explicit load or reject the simplification.
Keep identity moves in validation even when their physical transfers disappear.

Code home: [copies.rs](../src/mir65816/emit/copies.rs),
[select.rs](../src/mir65816/emit/select.rs), and their existing edge tests.
Allocation/staging validation must continue to agree with the emitted plan.

Acceptance: CRC32's identified true-arm/backedge round trips disappear except
for justified state repair. The false-arm transfer remains. Test mixed BYTE/
CARD/LONG groups, all-identity edges, swaps, rotations, repeated sources,
partial overlaps, malformed staging, fallthrough and branched edges. Include
cases where an apparent identity is overwritten by another destination.

### 2. Native word transfers inside mixed-width staged edges

Extend that checked fallback to copy complete two-byte values with one A16
load/store pair and four-byte values with two pairs, in both capture and commit
phases. Retain the complete staging barrier between phases. BYTE values remain
exact-byte transfers; existing specialized pointer/word plans take precedence.
Initially retain the old path for three-byte or unsupported mixed groups.

Use the existing private transfer machinery where its overlap and mode
contracts apply. Choose the new form only after including REP/SEP and any
terminal-state repair in its size/cycle cost. Do not reorder different logical
copies just to group widths. Frame slots and external traffic remain unchanged.

Acceptance: the remaining CRC32 four-byte transfers use native words where
profitable, with identical simultaneous-copy results. Exercise interleaved
1/2/4-byte groups, cycles, first/last BYTE copies, boundary displacements and
partial overlap. Report this saving after slice 1, not against the original
6.30-million-cycle identity cohort a second time.

### 3. Short 16-bit constant shifts in A

Extend [shifts.rs](../src/mir65816/emit/shifts.rs) before its scratch-based
constant-shift path. Start with exact two-byte private source/destination homes
and constant counts 1 through 7. Load A16 once, emit `ASL A` or `LSR A` for the
count, then store once. Reading the complete source before writing makes an
overlapping destination safe when both homes are otherwise valid.

Keep existing zero/count-at-or-above-width handling, byte-displacement
specializations, variable shifts and 24/32-bit chains. Preserve the compiler's
existing **logical** right-shift semantics, including INT bit patterns; do not
substitute an arithmetic right shift. No new input promotion rules.

Acceptance: the bit helpers' `>>3` no longer use RESULT scratch. CRC16's shift by
one also qualifies. Test counts 0/1/3/7/8/15/16/17 and large/negative count bit
patterns through fallback, source/destination overlap, sign-bit inputs and
both host/runtime modes. Extend existing `constant_shifts` coverage rather
than creating a separate broad arithmetic suite.

### 4. Direct long-indexed BYTE loads from allocated symbols

Extend [addresses.rs](../src/mir65816/emit/addresses.rs) for the narrow subset
already recognized as a symbolic base and a captured unsigned CARD index:
one-byte nonvolatile load, stride one, initially zero displacement/addend.
Load the index into X16 and use a relocated absolute-long indexed BYTE load.
Retain the current `[$dp],Y` path for dynamic pointer bases and other shapes.

Add the typed instruction form in
[selected.rs](../src/mir65816/emit/selected.rs), with index-aware conservative
memory effects, X consumption, correct M-dependent access width, state tracking
and o65 fixups. Do not describe the indexed access as an unindexed symbol read.
Preflight the original complete capture home and relocation before emission.

Reject this choice when an active loop-X contract needs X preserved; do not
introduce push/pop saves. Account for X clobbering in selection and replay.

Acceptance: sieve flags tests and bit-table reads avoid rebuilding the base in
DP. Use independent instruction encoding checks, flat and rebased o65 execution,
index boundaries 0/255/256/65535, data-bank crossings, neighbor canaries and
negative cases for volatility, signed/wider indexes, stride/displacement and
active X residency. Every external load remains exactly one BYTE.

### 5. Direct long-indexed BYTE stores

Reuse slice 4's checked address selection for one-byte stores. Admit numeric
BYTE constants and already captured private BYTE payloads. Establish X before
loading the payload into A8, then emit the relocated long indexed store. Keep
all source evaluation and reads at their original sites, including compound
assignments; this slice does not fuse read/modify/write operations.

Acceptance: standard sieve initialization and composite marking use this path;
bit sieve benefits where the same address shape is present. Test constant and
captured payloads, payload/index home overlap, high bank carry, read-only and
neighbor protection, relocated stores and X-residency fallback. Report loads
and stores separately; 659,001 cycles is their combined old setup cost.

### 6. Adjacent incoming CARD capture to comparison

Plan an immutable incoming two-byte parameter Load and its immediately adjacent
sole-use unsigned word Compare together. Feed the original parameter home to
native comparison selection and omit only the capture/store/reload work that
is no longer needed. Initially cover Eq/Ne and unsigned ordering; materialized
Boolean and fused-branch consumers must each retain their established contract.

Reuse the parameter eligibility checks in
[parameter.rs](../src/mir65816/emit/parameter.rs), complete use counting, and the
consumer-planning pattern in
[byte_consumers.rs](../src/mir65816/emit/byte_consumers.rs). Require the current
incoming home, complete disjoint ranges, unchanged stack depth and no mutable
frame object or address escape. Avoid a broader lifetime-forwarding analysis.

Acceptance: all three standard-sieve length captures can be eliminated when
their typed shape meets these conditions. Test both operand positions, boundary
values, extra uses including edge arguments, mutable/escaped parameters, calls,
intervening operations and near-limit stack offsets. Keep the incoming memory
read and comparison semantics; leave allocated temp slots intact.

### 7. BYTE/CARD top-bit mask to branch

Recognize an adjacent sole-use `AND` with the exact top-bit mask followed by an
Eq/Ne zero comparison whose result is used only by the block's branch. Consume
the original captured private value's N flag at its actual width and select
BMI/BPL. Plan the AND, Compare and branch together before omitting definitions.

Start with masks `$80` and `$8000`, both equality senses and commuted constants.
Keep other masks, returned/stored Boolean results, extra uses, nonadjacent
operations and external memory operands on their current paths. Normalize
types only from MIR facts; do not infer a BYTE sign from an A16 promoted value.

Acceptance: CRC8/16 top-bit tests no longer materialize the masked temporary.
Exhaust BYTE inputs, test CARD sign boundaries and poisoned hidden B, branch
inversion, joins, long branch relaxation and rejection cases. Restore the block
width contract without destroying the tested N flag or trusting ambient flags.

### 8. LONG top-bit mask to branch

Extend slice 7 to an exact 32-bit mask `$80000000`. Validate the complete private
four-byte operand, then load only its high word in A16 and branch on N. Reuse
the existing long-condition infrastructure in
[long_order.rs](../src/mir65816/emit/long_order.rs) where appropriate. Do not
change general LONG comparisons or split/reorder source-memory loads.

Acceptance: CRC32's AND-zero-low-word, masked-result stores and full zero test
disappear. Cover zero, `$7FFFFFFF`, `$80000000`, `$FFFFFFFF`, varying irrelevant
low words, both senses/constant positions, malformed or incomplete homes and
shared uses. Record actual replacement cycles; the full 2,752,142-cycle old
test is not a removable-work promise.

## Validation and measurement per commit

Run focused emitter/analysis/replay tests and the affected native runtime
targets. Existing consumers include `word_edges`, `acyclic_edges`,
`edge_coalescing`, `pointer_edges`, `constant_shifts`, `byte_indexed_accesses`,
`parameter_forwarding`, `compare_branch`, `byte_comparisons` and
`native_bitwise`; select from actual changes instead of running every target
after each slice. Cover raw and optimized source lowering in these regressions.

For each slice, rebuild and execute the affected CRC/sieve comparison in debug
and release hosts through `run_crc.py` or `run_sieve.py`. Keep unaffected
baseline reports. Run both comparison families together at the series endpoint.
Check complete outputs, ABI, guard policy, exact Action external access widths,
bank crossings, LF/CRLF compilation, mode transitions and IRQ/NMI reentry for
changed windows. New indexed forms also require relocation and independent
encoding tests. Document intentional private stack/DP traffic changes.

Save fresh builds under distinct ignored `target/` directories. Add compact
per-slice and cumulative results in a new series report, leaving the original
snapshots immutable. Record code/initialized-data bytes, guard-region bytes and
cycles, total cycles, stack peak, and correctness. Compare matching cases and
unchanged foreign-compiler machine bytes; never compare different input sizes
as a speedup. Remeasure rather than adding overlapping forecast percentages.

Use compile-only frozen-Exec footprint measurement to check the release budget
and identify any size regressions; do not infer it from these kernels. Guard
subtraction remains an accounting estimate, not a separately built unguarded
release: entry guard regions include necessary frame arithmetic.

**Do not run full/final backend or hosted Exec qualification**, including after
the last commit, under the user's standing instruction. No shared frontend/NIR
contract changes are planned; if a slice unexpectedly needs them, separate and
rescope that work with the corresponding shared-contract checks.

## Defer until these results are available

Reprofile before adding BYTE immediate arithmetic/bitwise selection, adjacent
shift-to-XOR result forwarding or further mode cleanup. Those are plausible
CRC8/16 follow-ups, but their incremental cost needs fresh measurement.
General loop invariant hoisting, alias-sensitive memory forwarding, helper
inlining, persistent register allocation and guard removal are outside this
series. The eight slices above should establish how much remains without
requiring a broader memory-access contract.
