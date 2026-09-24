# Native 65816 integer arithmetic helpers

Status: implemented and qualified on 2026-09-24.
Baseline: `c99cd07b`. Target: `wdc-65816-native`.

## Outcome and scope

Compile runtime integer multiplication, division and MOD into compiler-owned
native helpers, with the same typed arithmetic results as constant evaluation
and the other backends. Link only helpers that are used. Support fixed native
images and relocatable o65 applications, including calls from tasks and IRQ
dispatchers under the existing context bridge.

Use native 16-bit operations for the arithmetic core. Preserve physical ABI
`action65816.native.v1`, source integer promotion, explicit casts, argument
evaluation order, stack guards and per-domain ownership. Prioritize linked code
size, with execution cycles as the secondary measure. Record both; do not claim
a size or speed improvement before measurement.

This delivery includes constant power-of-two multiplication and unsigned
division/MOD selection for native 65816. REAL, small-model emission, foreign
arithmetic ABIs, changes to integer promotion, general constant-multiply
addition chains and cross-expression DIV/MOD fusion are separate work.

## Baseline integration points

| Area | Current behavior and required change |
| --- | --- |
| [Typed lowering](../src/mir65816/lower.rs) | Binary operations retain width and signedness. Explicit NIR faults are rejected for lack of a native Error adapter. Preserve those facts and add the selected arithmetic fault path. |
| [Native selection](../src/mir65816/emit/select.rs) | ADD/SUB, bitwise operations, comparisons and shifts execute; MUL/DIV/MOD reach an unsupported-operation diagnostic. Replace that boundary with verified helper calls or constant reductions. |
| [MIR and call plans](../src/mir65816/mod.rs) | Ordinary calls already carry argument homes, result lanes, outgoing extent and mode/stack contracts. Reuse this machinery for helper calls. |
| [Allocation](../src/mir65816/emit/allocation.rs) and [scalar DP promotion](../src/mir65816/emit/scalar.rs) | Calls affect live homes, scratch eligibility and local stack peaks. Helper calls must exist before these analyses run. |
| [Tracked emitter](../src/mir65816/emit/tracked.rs) and [physical effects](../src/mir65816/emit/effects.rs) | All instructions and calls have derived effects and checked selected-action records. Helper bodies and calls must use this boundary. |
| [Relocation](../src/mir65816/relocation.rs), [fixed images](../src/mir65816/image.rs) and [o65](../src/mir65816/o65/prepare.rs) | Code ownership, targets, placement, imports and maps currently cover user routines and the stack-overflow adapter. Add stable helper identities and the arithmetic-fault dependency to every consumer. |
| [Native qualification](../tools/native65816-runtime-tests/README.md) | Independent CPU execution, ca65 comparisons, stack guards, context switching and rebased o65 tests already exist. Extend these gates rather than introducing a separate emulator. |

The [lowering contract](MIR65816_LOWERING_CONTRACT.md) distinguishes retained
arithmetic facts from executable support. This plan extends executable support;
it does not move source-language decisions into emission.

## Implemented representation

The typed helper descriptor is stored on a compiler-owned `Mir65816Routine`.
This reuses routine identities, placement, maps and relocation ownership while
keeping `CallTarget::Helper` distinct from source routine calls. Generated
routines have canonical ABI metadata, no source span and no source-MIR blocks;
all physical instructions and branches have tracked effects and selected CFGs.
`MachineProgram.prepared` retains the exact verified graph used by allocation
and emission. Fixed-image linking checks it against idempotent preparation of
the supplied MIR, and o65 consumes it directly.

Source typing also permits SIZE operands in a narrow INT multiplication. The
MUL16 helper receives their low 16 bits; no 24-bit multiplication family is
invented. Narrow BYTE operands are explicitly zero-extended for MUL16. Casts
from the product to a consumer, including signed INT to LONGCARD, remain after
the operation.

Working scratch is D+$00..03 for the dividend/multiplicand, +$04..07 for the
divisor/multiplier, +$08..0B for product, +$0C..0F for remainder, +$10..11 for
quotient sign and +$12..13 for remainder sign. Only the needed words are used.
X16 is the 16/32-iteration counter; helpers make no nested calls or pushes.
The entire $00..3F ABI scratch region remains caller-clobbered.

## Arithmetic contract

Select from the verified operation's width and signedness, after its explicit
operand conversions. The destination storage width does not choose the helper.
Conversion to a narrower or wider consumer happens after the operation.

| Family | Required computation and result |
| --- | --- |
| MUL16 | Low 16 product bits. This also serves widened BYTE/CHAR products under the existing INT multiplication rule. Signed and unsigned bit patterns use the same modular core. |
| MUL32 | Low 32 product bits for LONGINT/LONGCARD operations. |
| Unsigned DIV/MOD8 | BYTE/CHAR quotient or remainder, zero-extended in the ABI result. |
| Unsigned DIV/MOD16 | CARD-domain quotient or remainder. |
| Signed DIV/MOD16 | INT-domain quotient or remainder. |
| Unsigned DIV/MOD24 | Legal SIZE-domain operations. Use zero-extended 32-bit working values initially and return the exact 24-bit result. |
| Unsigned DIV/MOD32 | LONGCARD-domain quotient or remainder. |
| Signed DIV/MOD32 | LONGINT-domain quotient or remainder. |

The width inventory in slice 1 must confirm which operations source typing
actually produces, including SIZE. Do not invent byte-result multiplication,
signed 24-bit arithmetic or pointer multiplication to fill a symmetric table.
For example, current narrow multiplication has an INT result even when its
operands are bytes; assignment to LONGCARD does not retroactively widen it.

Use the contract in [semantic integer arithmetic](../src/semantic/integer.rs):

- Unsigned division returns the floor quotient and a remainder below the divisor.
- Signed division truncates toward zero. A nonzero remainder has the dividend's
  sign; `r = a - q*b` for representable quotients.
- Signed MIN divided by -1 wraps to MIN, with remainder zero, at each supported
  signed width. Magnitude conversion must handle MIN without host or target
  signed overflow assumptions.
- Literal/foldable zero divisors keep their existing frontend diagnostic.
  Runtime zero causes a deterministic non-returning DivisionByZero fault,
  including operations whose result is unused.
- Calls, indirect loads and volatile reads happen once in their existing order.
  No helper reloads an original source address after NIR has captured its value.

## Helper representation and call interface

Introduce a native helper descriptor table keyed by a typed identity containing
operation, computation width and signedness. Display/link names are metadata;
selection and relocation must not infer contracts from strings. Suggested code
homes are `src/mir65816/arithmetic.rs` for planning/legalization and
`src/mir65816/emit/arithmetic.rs` for helper bodies.

Run one shared native preparation step before frame allocation and instruction
selection. It first performs eligible constant reductions, then replaces the
remaining MUL/DIV/MOD operations with ordinary `Mir65816Op::Call` operations
targeting a new typed helper target. Build each `Mir65816CallPlan` using the
existing ABI layout functions. Keep explicit arithmetic faults as non-returning
operations, not returning calls with a discarded result.

The prepared program must own its helper descriptors. Fixed-image and o65
linking, verification, maps and emission must consume that same prepared program;
do not mutate a private emitter copy while linking the original call graph.
Existing typed MIR can retain Binary forms before this preparation boundary.
Verify helper IDs, argument conversions, result widths, plans and dependency
closure again after legalization. A second preparation must be idempotent.

Helpers use the ordinary native stack ABI, with two read-only value arguments
and one scalar result. For equal-width operands the layouts are:

| Operand bytes | Argument offsets | Outgoing bytes O | Result home |
| ---: | --- | ---: | --- |
| 1 | 0, 1 | 3 | A low byte, A high byte zero |
| 2 | 0, 2 | 5 | A16 |
| 3 | 0, 4 | 7 | A16 plus X low byte, X high byte zero |
| 4 | 0, 4 | 9 | A16/X16 |

At a zero-frame helper's entry the arguments start at S+4+offset. The caller
checks O plus the three JSL return bytes before reserving or writing anything,
zeros padding and releases O after RTL. Use the existing result-preserving
cleanup. Include helper calls in frame/stack maps and displacement validation.
Retain the checked helper entry even for a zero-frame body; qualify its zero
reservation check and any later reservation with the same stack-bound rules.

Normal helper entry/return has E=0, M=X=0, decimal clear, DBR=0 and unchanged
D/I. A/X/Y and arithmetic flags are clobbered; S is balanced. Do not introduce
implicit register arguments, fixed absolute zero-page cells or unchecked JSR
shortcuts. Emit with JSL/RTL so helpers and callers may occupy different banks.

Allocate working state only within the existing D+$00..$3F scratch region or
checked invocation storage. Prefer zero-frame leaf helpers with bounded loops
and no further calls. Publish each body's exact scratch map, modes, stack peak
and clobbers. D+$40 and above remain protected domain metadata/reservations.
The [physical ABI](MIR65816_PHYSICAL_ABI_V1.md) already permits all 64 scratch
bytes to be clobbered by an ordinary call; allocation must enforce that boundary
even when a particular helper touches fewer bytes.

This is especially important for live word temps promoted into D+$20..$3F,
pointer leaves, resident X loops and adjacent accumulator forwarding. Reject
incompatible residency or use the existing call-safe stack path. No live caller
value may survive solely in helper scratch or caller-clobbered registers.

## Arithmetic bodies

1. **Multiplication:** bounded shift/add, using a native word for the 16-bit
   core and two words for the 32-bit core. Keep only the resolved result width;
   initialize every carry chain explicitly and preserve carry between limbs.
   Start without large lookup tables or data-dependent early-exit variants.
2. **Unsigned division/remainder:** a restoring division core computes quotient
   and remainder. Retain the extra remainder bit needed when shifting a value
   near the maximum divisor; a width-sized remainder register alone must not
   lose that carry. Test divisors with the top bit set explicitly.
3. **Signed division/remainder:** capture the original signs, convert to unsigned
   magnitudes modulo the computation width, run the unsigned algorithm, then
   correct quotient and remainder signs independently. Correct sign handling
   must include zero remainder and MIN/-1.
4. **Byte and 24-bit forms:** reuse a qualified wider algorithm where profitable,
   with explicit zero extension and exact ABI result masking. Read only the
   declared argument bytes; padding is not part of an operand.

Share algorithm generation between DIV and MOD. The initial implementation may
emit separate entry bodies to keep the call contract straightforward. Sharing
executable cores is an optional measured follow-up that must preserve ordinary
entry/return and scratch ownership; it must not expose a second result in
undocumented scratch. Fusion of two source operations is outside this delivery.

Use the tracked emitter for every instruction. Add any missing native word
shift/rotate forms with encoding, effect, state-tracking and replay tests.
Do not insert opaque opcode blobs that bypass instruction effects or selected
control-flow validation. Check branch relaxation and whole-helper placement
near bank ends; execution must never depend on PC wrapping into another bank.

## Division-by-zero adapter and artifact compatibility

Add a distinct raw non-returning platform entry,
`__a816_arithmetic_fault_v1`. Proposed entry contract:

- A16 contains native reason 1 = DivisionByZero. This is an explicit native
  adapter code, not an enum ordinal or Atari Error number.
- X16 contains S at the fault transfer; Y and arithmetic flags are unspecified.
- E=0, M=X=0, decimal clear, DBR=0; D and I remain those of the failing domain.
- Transfer uses JML with no additional stack push. The adapter cannot RTL/RTS;
  it terminates or hands control to the platform's fault policy using a declared
  workspace/stack. It may change IRQ state after accepting the transfer.
- S may include the caller's helper arguments and JSL return address. This is
  not an unwind or resumable-error interface. No result store or subsequent
  source operation is executed.

Require and validate the adapter only when a surviving helper or explicit
DivisionByZero fault needs it. Constant reductions that prove the divisor
nonzero add no dependency. Optimizer-generated `NirCallee::Fault(DivisionByZero)`
must reach the same adapter; support cannot depend on whether NIR folded the
operation. Other native runtime-fault kinds keep their explicit diagnostic.

Use a separate typed relocation target and a terminal selected-action contract.
The existing stack-overflow target cannot stand in for an arithmetic fault:
its A register, meaning and provider contract are different.

Fixed-image layout input needs an optional arithmetic-fault address, made
mandatory when used, and the serialized image must retain enough information
for a loader to validate the dependency. o65 needs a named raw import contract
distinct from both ordinary returning routines and raw stack overflow. Validate
provider identity, domains, address/extents and non-returning behavior metadata;
reject missing or incompatible providers before execution.

Version the new serialized fault contract explicitly in slice 2: use image v4
and a new experimental o65 profile version for artifacts that require it, while
retaining v3/current-profile support for existing artifacts. Old readers must
reject the new feature, rather than silently treat it as a normal import.
The physical call layout and ABI identity remain v1. Document this additive
platform interface and regenerate ABI constants if its manifest is extended.

## Constant arithmetic selection

Before helper legalization, recognize typed constant operands in native MIR:

- Multiplication by 0/1 or a power of two becomes zero/copy/logical left shift at
  the resolved multiplication width. Preserve prior operand evaluation.
- Unsigned division by 2^k becomes logical right shift; MOD becomes AND with
  2^k-1. Division by 1 copies the value; MOD 1 yields zero after evaluation.
- Use typed constants after conversions, including constants widened to
  LONGCARD. Never truncate a divisor to the assignment destination's width.
- Signed DIV/MOD retains the general helper. Logical shifts and masks do not
  implement negative signed quotient/remainder semantics.

Keep these target choices within MIR65816 so this work does not silently change
other backends or source/NIR contracts. First admit verified immediate forms;
constant propagation remains NIR's responsibility. Exercise both raw and
optimized NIR. Operations not reduced must still compile through helpers in
either mode. Choose compact inline shifts or the existing bounded shift loop
using measured code size; helper removal alone is not a performance result.

## Initial body measurements

Measured with the pinned qualified CPU and independent ca65 callers, in both
incoming I states. Body cycles include the checked zero-frame entry and RTL;
caller cycles also include argument setup, JSL, result stores and cleanup.
These are representative input costs, not worst-case timing bounds. Operands
are `$F3/$0D` for byte, `$ABCD/$FFF1` for word, `$ABCDEF/$FFF1` for SIZE,
and `$8123ABCD/$0000FFF1` for long (signed families reinterpret those bits).
All bodies have a zero-byte frame. Stack below the assembly caller's initial S
is exactly O+3; no helper-local push is observed. Scratch counts below are
actual distinct written bytes within the published scratch map.

| Helper | Body bytes | Native call bytes | Body cycles | With ca65 caller | Stack bytes | DP bytes written |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| div_u8 | 109 | 71 | 652 | 732 | 6 | 6 |
| mod_u8 | 109 | 71 | 652 | 732 | 6 | 6 |
| mul_u16 | 66 | 80–82 | 587 | 693 | 8 | 6 |
| div_u16 | 94 | 80 | 585 | 691 | 8 | 6 |
| mod_u16 | 94 | 80 | 585 | 691 | 8 | 6 |
| div_i16 | 138 | 80 | 814 | 920 | 8 | 10 |
| mod_i16 | 138 | 80 | 826 | 932 | 8 | 10 |
| div_u24 | 142 | 97 | 2115 | 2247 | 10 | 12 |
| mod_u24 | 142 | 97 | 2115 | 2247 | 10 | 12 |
| div_u32 | 127 | 106 | 2123 | 2281 | 12 | 12 |
| mod_u32 | 127 | 106 | 2123 | 2281 | 12 | 12 |
| mul_u32 | 89 | 106 | 1571 | 1729 | 12 | 12 |
| div_i32 | 192 | 106 | 2341 | 2499 | 12 | 16 |
| mod_i32 | 192 | 106 | 2341 | 2499 | 12 | 16 |

The 24-bit forms use the 32-bit core; extra copies and result masking explain
their code overhead. Separate DIV and MOD bodies intentionally duplicate the
core. The existing emission snapshot remains unchanged for unrelated kernels.
Detailed operands, scratch offsets, observed stack and cycle counts are saved
in each qualification run's `arithmetic-metrics.json`.

Native call bytes cover the complete selected helper-call span, including the
stack check, padding/payload writes, JSL, cleanup and result capture. Operand
loads/casts outside that span are excluded. Raw and optimized results agree;
MUL16's 80–82-byte range reflects preceding accumulator mode. A repeated DIV
adds another call span and no helper body. DIV plus MOD emits two bodies.
Reproduce this inventory with `A816_ARITH_COST_PATH=/tmp/arithmetic-calls.json
cargo test --test mir65816_arithmetic` (one shell command).

Observed cycle ranges for optimized complete kernels, including the ordinary
Main caller and result stores, are below. MUL includes its consumer widening
to LONGCARD. DIVMOD computes both quotient and remainder. Byte DIVMOD covers
all nonzero divisor pairs; wider inputs cover boundary cross-products and 1024
seeded pairs. These measured maxima are corpus maxima, not timing guarantees
for arbitrary call sites. Raw-mode ranges and maximizing inputs are retained
in `arithmetic-range-*.json`.

| Source type | MUL kernel cycles | DIVMOD kernel cycles |
| --- | ---: | ---: |
| BYTE | 736–841 | 1552–1936 |
| CARD | 706–915 | 1554–2322 |
| INT | 706–915 | 1628–2408 |
| SIZE | 768–977 | 3722–6218 |
| LONGCARD | 1603–2403 | 3706–6778 |
| LONGINT | 1603–2403 | 3780–6895 |

## Qualification checkpoint

All seven implementation slices are complete. Checks passed:

- 196 native library tests with selected-state proofs; one opt-in test ignored.
- 68 native integration checks, plus the final call-cost/deduplication check;
  four inventory exporters remain opt-in. The reviewed emission snapshot is
  unchanged. Arithmetic and o65 contract rejection tests pass in both modes.
- Full VM qualification: 168 passed in debug and 168 in release; four external
  comparison/inventory tests remain opt-in. Final arithmetic-only reruns passed
  all 10 tests in both profiles, adding explicit cross-bank caller/helper
  execution and timing-range recording after the full runs.
- The final full-suite arithmetic preemption probe validates every reached
  helper instruction in both task domains with IRQ and NMI restoration, plus
  deterministic mixed schedules. Its dispatcher runs the same 14 helper
  families in its own domain. Seeded arithmetic IRQ frequency is 1/1024
  instruction boundaries, NMI 1/4096 with the existing 250-cycle cooldown;
  this permits task progress while IRQ dispatch runs the full arithmetic set.
- Inputs are checked before/after qualification; LF/CRLF context-source
  transformations agree. No shared NIR or other backend contract changed.

Qualification artifacts remain under `tools/native65816-runtime-tests/target/qualification/`.
The manifests retain source hashes, VM pin/patch, tool versions and artifact
hashes. SHA-256 of the checkpoint manifests:

| Run | Manifest SHA-256 |
| --- | --- |
| Full debug: `run-_ekjl67c` | `90f428d586e52162b5437abcc04acd8d7df38b22783b7eca1deb7c8e726a6f39` |
| Full release: `run-ajl0c0zk` | `dd82fa2a595a00055fd8c060a91913b2e5a2dab50778bb981ae0a0e39abf7606` |
| Final arithmetic debug: `run-8zs26dc5` | `0c09b3c45bfa9d5a57fefaf6109a999e4eed1d13250eaf4073f1ce33cd8e1857` |
| Final arithmetic release: `run-qr10lqp4` | `cbd63b9058ed5a18f33a77ee331b2cda60ff97e46ed136c90e741b17e0182430` |

## Implementation slices and acceptance

| Slice | Deliverable | Gate before proceeding |
| --- | --- | --- |
| 1. Contract and representation | Freeze legal width/promotion probes, typed helper IDs/descriptors, ordinary call plans, scratch ownership and the non-returning fault contract. Add legalization before allocation and verification tests without advertising unsupported bodies. | Incorrect IDs, widths, plans, effects and illegal targets fail; unrelated native programs retain their emitted code and allocation. |
| 2. Native fault and linking | Add the native fault target, explicit NIR DivisionByZero handling, versioned fixed-image/o65 contracts, loader validation and a qualification adapter. | Runtime and optimized-known zero reach the same adapter with exact A/X/S/modes; no following store executes; missing/incompatible providers fail. |
| 3. Word helpers | Implement MUL16 and signed/unsigned DIV/MOD16 through the complete compiler, ABI and linker path. | Boundary cross-products, sign quadrants, MIN/-1, mixed promotions, nested expressions, calls and live values across helpers pass independent execution. |
| 4. Remaining widths | Add unsigned byte division/MOD, MUL32, signed/unsigned DIV/MOD32 and legal unsigned SIZE forms. | Exhaustive byte pairs; wide boundary/random oracles; result truncation/extension and source-read traces pass. |
| 5. Constant selection | Add native power-of-two reductions before helper demand collection. | All relevant exponents, 0/1 multiplication, divisor 1, cast constants and effectful operands pass; reduced-only programs link no arithmetic helper or arithmetic-fault dependency. |
| 6. Context and relocation qualification | Exercise the same helper text in two tasks and IRQ dispatch, stack boundaries, multiple code/data placements and rebased o65. | Exhaustive reachable helper instruction boundaries plus seeded IRQ/NMI schedules preserve state, results, domain scratch and canaries. |
| 7. Measurement and documentation | Publish helper/call-site bytes, cycles, stack peaks, artifact hashes and advertised support; update lowering/emission/acceptance and runtime-test documentation. | Full native debug/release qualification passes, focused compiler checks pass, and every advertised width/operation has executable coverage. |

Keep each slice reviewable. Do not enable a helper family before its fault,
linking and ABI path is complete. Pure MUL support can be tested independently,
but complete DIV/MOD support requires slice 2.

## Validation matrix

- **Numeric:** all 256x256 unsigned byte operand pairs, with zero handled by the
  fault oracle; word boundary cross-products; deterministic randomized word and
  long pairs; zero, one, maximum, sign boundaries, every power of two, divisors
  larger than dividends, equal operands and negative-zero-remainder cases.
  Compare executed code with independent widened host arithmetic/magnitude-sign
  oracles. Also check the shared semantic evaluator, without using a duplicate
  of the target loop as the only oracle.
- **Source integration:** assignments, function results/arguments, compound
  assignments, indexed/pointer destinations, mixed signedness, narrow/wide
  consumers, reused/unused results, recursion and nested operand calls.
  Supply inputs after compilation to prevent constant folding from replacing
  the helper under test. Check operand evaluation counts and exact volatile
  byte reads, including MMIO-style aliases and bank-crossing loads.
- **ABI:** independent ca65 callers and callee probes; all input/result widths,
  padding and unused result bits; both incoming I states; full D/DBR/S and
  boundary mode checks; live caller values across a helper that clobbers all
  allowed scratch; stack-floor, ceiling and subtraction-underflow tests.
- **Interrupts:** both tasks call identical helper text while using distinct D
  blocks; IRQ dispatch uses its own block and calls that helper too. Interrupt
  carry chains, sign conversion, divide loops, result assembly and call cleanup.
  Check full resumed register/status state. NMI follows the existing bounded
  assembly-only policy and never calls an arithmetic helper.
- **Artifacts:** one emitted body per demanded helper identity; no unused bodies
  or mutable global workspace; readable helper code ranges and names in maps
  without invented source locations; deterministic builds; relocated helper
  calls/fault targets, invalid bindings and code-bank-end placements. Execute
  from serialized fixed images and o65 files, not compiler-owned memory objects.
- **Regression and cost:** record linked text and per-call bytes, typical and
  worst-case cycles, exact local/call-chain stack peaks and DP traffic. Include
  one use versus repeated uses and DIV plus MOD together. Unrelated native
  kernels keep their code bytes; versioned metadata changes are accounted for
  separately. No guard removal is part of this work.

Add focused compiler tests (for example `tests/mir65816_arithmetic.rs`) and a
native VM target `arithmetic_helpers`. Extend the existing `interop`,
`stack_faults`, `preemption` and `o65` targets where they own the behavior.
Use the repository's [qualification runner](../tools/native65816-runtime-tests/qualify.py),
not bare Cargo in that isolated VM workspace:

```sh
cargo test --lib mir65816::
cargo test --test mir65816_arithmetic --test mir65816_abi --test mir65816_contract --test mir65816_emission
cargo test --test mir65816_o65 --test actionc_65816_cli --test actionc_65816_o65_cli
python3 tools/native65816-runtime-tests/qualify.py --test arithmetic_helpers --test interop --test stack_faults --test preemption --test o65
python3 tools/native65816-runtime-tests/qualify.py
python3 tools/native65816-runtime-tests/qualify.py --release
```

The new test-target names above become runnable in their implementing slices.
Run ABI generation checks if the manifest changes. Cover LF/CRLF through any
new source/assembly fixture instrumentation. Keep compiler and fixture inputs
unchanged during qualification so its provenance checks remain valid.

These are native-backend changes. Follow [AGENTS.md](../AGENTS.md) for scoped
checks; do not run unrelated backend suites by default. If implementation
changes shared semantic/NIR contracts, additionally run the required NIR
snapshots, fixture sweep and full compiler suite, and audit all consumers.

## Completion record

Implementation is complete only when the width matrix, zero-fault path,
constant reductions, fixed-image/o65 linking and context qualifications above
are delivered. Record final commits, qualification manifests and measured costs
here, and update [the backlog](BACKLOG.md). Do not mark the feature complete
based on isolated arithmetic-core tests or retained MIR operations alone.
