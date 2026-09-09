# Modern integer arithmetic implementation plan

Status: implemented, including the initial bounded slice-6 selectors and slice-7 ports.
Created 2026-09-06. Baseline: `5777b0f`.

The [legacy arithmetic audit](bugs/LEGACY_INTEGER_ARITHMETIC_AUDIT.md) records
the executable evidence and open defects. This plan replaces accidental
arithmetic behavior with a shared language contract and generalizes existing
helper selection. It is not a benchmark-specific optimization plan.

## 1. Scope and agreed boundaries

- Every actionc profile, including Compatibility, uses the same correct
  integer arithmetic semantics. Modern-only language syntax may remain gated.
- The user-planned original-compiler-in-VM mode is the route to historical
  behavior. Implementing that mode is outside this plan.
- Existing cartridge-linked output remains supported. A program can call
  cartridge library services and also contain compiler-owned arithmetic helpers.
  `Runtime::ActionCart` selects services/linking, not a numeric dialect.
- Classic and MIR6502 must agree under both runtimes. Preserve the arithmetic
  contract through the 68k/65816 lowering canaries as well; do not claim native
  executable support before their backends implement it.
- Correctness comes before specialization. Reuse SemIR typing, typed NIR
  operations, MIR selectors, runtime bindings, and selective linking.
- Implement compiler repairs separately from Oscar64 translations. Do not
  rewrite test expressions to hide a compiler failure.

The first delivery covers integer division/remainder, their interaction with
multiplication, constant evaluation, and ordinary/compound consumers. It does
not change REAL arithmetic, integer widths, pointer widths, public routine
ABIs, general expression evaluation order, or the full shift/addition policy.

## 2. Implemented numeric contract

The rules below apply to Compatibility, Optimized classic, and MIR6502, with
both cartridge and standalone linking. Multiplication keeps its existing INT
result type; the possible unsigned-product typing change is deferred.

### Types, conversion, and result interpretation

Keep BYTE/CHAR unsigned 8-bit, CARD unsigned 16-bit, and INT signed 16-bit on
all targets. A 32-bit 68k register or a 16-bit 65816 accumulator does not change
the source type's range.

For division/remainder, retain the existing common-type precedence initially:

| Operand combination | Arithmetic domain |
| --- | --- |
| BYTE/CHAR with BYTE/CHAR | Unsigned 8-bit; retain existing BYTE/CHAR result-kind rules |
| INT with INT or BYTE/CHAR | Signed 16-bit |
| CARD with any integer type | Unsigned 16-bit |

Normalize conversions explicitly. In the CARD domain, a negative INT converts
by its modulo-65536 bit pattern; it is not treated as a negative unsigned
dividend by the helper. Programs wanting signed interpretation use an
explicit INT conversion. Do not introduce implicit 32-bit source promotion.

The destination does not select the arithmetic domain. Compute first, then
apply the existing final store conversion. For example,
`BYTE result; CARD divisor=256` must not turn division by divisor into division
by zero. Preserve exact existing operand evaluation and compound captured-place
ordering; a helper optimization cannot duplicate a call or hardware read.

### Division, remainder, and exceptional inputs

- Unsigned: `q=floor(a/b)`, `r=a-q*b`, with `0 <= r < b` for nonzero b.
- Signed: quotient truncates toward zero; `r=a-q*b`; nonzero r has a's sign
  and `abs(r) < abs(b)`. MOD is a remainder operation, not Euclidean modulo.
- Overflow rule: `INT_MIN/-1` returns INT_MIN by explicit
  wrapping, and `INT_MIN MOD -1` returns zero. This preserves the existing
  wrapping edge already exercised by runtime tests. The mathematical identity
  above applies to representable quotients; this exceptional quotient obeys
  the modular identity instead.
- Required constant expressions that divide by zero produce a diagnostic.
  Also diagnose literal/foldable zero divisors in executable source expressions
  after typed conversion, including `BYTE(256)`. Report the source operation,
  never a Rust panic.
- Dynamic-zero rule: a deterministic, non-returning arithmetic
  fault, with no normal result and no following source effects executed.
  The target-independent kind is `ArithmeticFault::DivisionByZero`. On Atari
  6502, the helper calls existing Error with A=101, X=0, Y=101 (cartridge `$04CB`
  or linked standalone SYSLIB Error). Code 101 is actionc's dedicated extension,
  replacing the initial generic-100 fallback; original division has no zero check
  or dedicated error code. If Error returns, the helper clears decimal mode,
  restores A/Y=101, sets carry, and enters a `BCS` self-loop. No normal return
  or following source effect occurs. This is not exception unwinding or
  recovery. Each native emitter must define its delivery before executable
  support is enabled. See [Atari runtime errors](ATARI_RUNTIME_ERRORS.md).
- Optimizer-discovered zero in a reachable runtime computation becomes that
  fault, not a whole-program compile error inferred from an unreachable path.
  Unreachable runtime paths do not fault. Statically invalid constant syntax
  remains a frontend diagnostic independently of runtime reachability.

All folds, static initializers, layout/CONST consumers, ordinary expressions,
and compound operations must use the same typed arithmetic rules where that
operation is legal. Checked object-layout size arithmetic remains distinct
from intentionally wrapping scalar arithmetic.

### Multiplication decision: do not change it accidentally

Current multiplication always has an INT result, and NIR verifies that rule.
The generated unsigned byte multiply returns a complete 16-bit bit pattern;
it does not by itself change that pattern's source-language interpretation.

The preferred longer-term modernization is CARD results for unsigned products
(including widened BYTE/CHAR products), INT for signed products, and an
explicit mixed-type rule. This is a source-language change: assignments,
overload/argument compatibility, comparisons, and subsequent division can
change even when emitted multiplication bits are identical.

Before adopting that change, settle the mixed-type/result table and audit
affected callers. If it is not explicitly included at slice 2, retain the
current multiplication result types for this delivery and track the change
separately. The division evaluator must then consistently interpret an INT
product as signed. Do not leave its constant form unsigned.

This decision is not permission to preserve wrong CARD division or broken
MOD in any profile. It distinguishes deliberate typing policy from defects.

## 3. Architecture and existing machinery to extend

### SemIR and typed constant evaluation

SemIR owns operator meaning, effective operand/result types, conversions,
and source sequencing. Build one typed integer evaluator around those facts,
with explicit outcomes: value, not constant, and arithmetic fault. Use wider
host intermediates or unsigned magnitudes so INT_MIN and overflow do not
trigger host-language panics.

Route the relevant evaluators in `semantic.rs`, `semantic/ir.rs`,
`nir/lowerer.rs`, `nir/optimizer.rs`, `codegen.rs`, and
`codegen/data.rs` through that implementation or a shared lower-level
primitive. Preserve separate address/layout eligibility checks. Do not
globally change every `u16` utility into signed arithmetic.

Classic receives resolved arithmetic facts through its existing SemIR
projection. NIR/MIR must not reconstruct type meaning from source spelling,
helper names, or the assignment destination.

### NIR: semantics and fault effects

Reuse typed Binary/Cast operations. Division/remainder must have an
unambiguous effective domain with verified operand conversions. Prefer the
existing type facts to redundant attributes; if an explicit domain is needed,
verify that it agrees with those facts.

Model potential arithmetic faults conservatively. Current NIR purity treats
all Binary operations as discardable. Audit dead-temp/store elimination,
propagation, common-subexpression handling, and movement across calls/control
flow. An unused result does not justify deleting an executed division that
may fault. A nonzero proof or a successfully evaluated constant can establish
safety; no broad range-analysis project is required for the initial fallback.

Keep a runtime fault observable to subsequent lowering. Do not hide it in an
apparently pure helper call. Start conservative and strengthen proofs later.
Verify before and after every optimization pass.

### MIR: target strategy, not semantic recovery

Retain signed/unsigned domain through 6502, 68k, and 65816 binary lowering.
Use typed operation variants or a small semantic domain field; reject
division/remainder with missing or inconsistent domain information.
Do not weaken verification to allow ambiguous legacy forms.

MIR owns native instruction choice, helper selection, operand preparation,
physical result locations, scratch resources, and cost. Preserve the existing
discarded-high-product/low-bit proofs for multiplication where applicable.
Division/remainder do not inherit multiplication's truncation rules.

### Typed helper contracts

Extend the existing helper declaration, selection, call-home, effect and
binding structures; do not introduce a parallel compiler/runtime registry.
The current `MirRuntimeBinarySelection` has one shared operand width and one
result width. Generalize it to independent argument and result contracts.

Each helper candidate must identify:

1. Logical operation/domain, including full product versus retained low bits,
   and quotient/remainder availability.
2. Each input's type/width and required conversion or proven range.
3. Each output's type/width, physical home, and adaptation to the semantic result.
4. Nonzero/range preconditions and fault/overflow behavior.
5. Registers, flags, memory regions, private scratch, stack effects and
   non-returning behavior where relevant.
6. Implementation binding and cost: call-site bytes/cycles, conversion/staging
   overhead, helper body/dependency size and existing linkage amortization.

Internal signatures need not equal the public Action! calling convention.
Keep public cartridge/library ABI placement unchanged and use the existing
ABI machinery to marshal private helper arguments.

Illustrative candidates, not promises to implement every variant:

| Candidate | Inputs | Results |
| --- | --- | --- |
| Widening unsigned multiply | u8, u8 | u16 product |
| Low-word multiply | u16, u8 | low 16 product bits |
| Unsigned divmod | u8, u8 | u8 quotient, u8 remainder |
| Unsigned divmod | u16, u8 | u16 quotient, u8 remainder |
| Unsigned divmod | u8, u16 | u8 quotient, u8 remainder |
| Unsigned divmod | u16, u16 | u16 quotient, u16 remainder |
| Signed divmod | i16, i16 | i16 quotient, i16 remainder |
| Signed dividend / unsigned byte divisor | i16, u8 | i16 quotient, i16 remainder |

The last remainder cannot generally be a signed byte: its magnitude can reach
254. BYTE/CHAR remain unsigned source types; signed narrow helper inputs, if
ever useful, require explicit representability proofs.

Narrow outputs may be extended to the source result type. Narrowing a result
does not justify narrowing an input: u16/u8 still needs a word quotient, and
u8/u16 must retain a divisor above 255 even though both results fit a byte.
Reject an unproven specialization by taking the correct general helper.

### Cartridge-assisted and standalone binding

First select the semantic operation and signature; then bind an implementation.
Both link modes initially use compiler-owned division/remainder. Do not reuse
legacy `RemI` or rely on its private post-call workspace. A shared unsigned
core with signed adapters is a natural starting point.

Reuse selective linking to include only demanded routines and dependencies,
once per program. Do not import all of SYSLIB to obtain one helper, patch ROM,
or mutate the original compiler's vectors globally. Cartridge services remain
available through their established public interfaces.

The generated `MulByte` binding already demonstrates a compiler-owned helper
independent of runtime choice. Generalize availability to shared machinery
consumed by classic and MIR, rather than leaving correct arithmetic as a
MIR-only facility. Retain audited existing multiply paths until their contract
or profitability requires a change.

Legacy math `SET` overrides need explicit migration handling. Known old ROM
bindings cannot silently replace the new operators. Diagnose unsupported
overrides when used, or provide an explicit typed custom-helper contract with
ABI/effect validation. A signature check cannot prove arbitrary user machine
code implements the mathematics: such overrides are trusted low-level
contracts, not automatically certified implementations. Raw fixed-address
procedure calls remain separate from compiler operator selection.

Later ROM reuse is optional and requires range/equivalence evidence and a
cost benefit. Cartridge-linked output must never select a different numeric
contract just to avoid linking a helper.

## 4. Implementation slices

Each major slice should be independently reviewable. When committing the
implementation, separate unrelated repairs, semantic rollout, optimization,
and test ports. Section 7 records the implemented scope and remaining limits.

### Slice 1a: eliminate constant-evaluation host panics

Repair ARITH-ZERO-PANIC without inventing a legacy arithmetic dialect.
Replace eager guarded evaluation, carry failure to a source diagnostic, and
audit sibling constant/address evaluators. Cover division and MOD, nested
arithmetic, FOR steps, conversions to zero, CONSTs and initializer contexts.
Runtime-dependent divisors must remain runtime expressions.

Acceptance: public compile APIs return diagnostics rather than unwind in all
profiles/runtimes; nonzero controls keep their current behavior. Do not use
`catch_unwind` in the compiler as the fix.

### Slice 1b: repair captured-operand fact lifetime

Fix ARITH-CLASSIC-CAPTURED-RELOAD independently of signed division semantics.
Retain the exact positive-input reproducer and correct oracle. Trace stored
memory facts and register aliases across pointer preparation, restoration,
stores, and later captured loads. Generalize invalidation/proof boundaries;
do not special-case MOD, the fixture's addresses, or INT/CARD source spelling.

Acceptance: all four positive pairs in the audit pass under all six
mode/runtime combinations. Extend coverage to distinct captured destinations,
byte/word loads, different high bytes, intervening calls, and ordinary/compound
consumers. Preserve once-only evaluation, input bytes, guards and stack balance.

### Slice 2: settle the contract and add the shared evaluator

Finalize division rounding, remainder sign, overflow, fault behavior and
mixed-type conversions from section 2. Explicitly record whether multiplication
result modernization is included or deferred. Do not infer that decision from
the selected helper's signature.

Implement and unit-test the shared typed evaluator. Initially expose it to
focused internal tests without changing only the public folding path while
runtime operators still disagree. Inventory old arithmetic folds and their
address/layout-specific responsibilities.

Acceptance: independent host oracles cover all 8-bit pairs and a systematic
signed/unsigned word boundary grid, including INT_MIN/-1 and converted zero.
Tests distinguish values, nonconstants and faults; legal constant contexts
agree. Document any intentional source type change and its migration impact.

### Slice 3: preserve domains, effects and helper signatures end to end

Carry the contract through SemIR projection, NIR verification, each MIR's
binary form, and helper discovery. Add the per-input/per-result contracts and
explicit result homes. Audit fault-sensitive optimizations before enabling
faulting operators publicly.

Initially make descriptor-driven general helpers and existing MulByte work;
do not implement every candidate or add speculative division optimization.
Reuse stable identities and existing call/effect verification. Account for
different physical signatures in materialization and linker validation rather
than comparing two copies of one generic ABI template.

Acceptance: signed and unsigned divides remain distinguishable in all target
IRs. Malformed domains, widths, result homes or bindings are rejected.
NIR tests retain executed faults when results die and do not introduce faults
on unexecuted paths. Existing MulByte selection and full-word results remain
correct; documented IR changes have focused snapshots.

### Slice 4: implement and directly validate correct helper bodies

Add a compiler-owned full-range unsigned word division core and signed
adapters. Preserve quotient/remainder independently through sign correction.
Represent INT_MIN magnitude as unsigned 32768, not an overflowing signed abs.
Handle the chosen wrapping edge explicitly.

Provide the chosen arithmetic-fault binding under cart and standalone linking.
Define its observable kind, non-returning control flow, workspace and platform
delivery before public enablement. Use a deterministic test binding to check
fault paths without depending on an interactive cartridge error screen.
No caller may resume normally from that handler.

Expose quotient-only and remainder-only demands using the existing linking
and call paths, even if their implementation shares a divmod core. Describe
all physical outputs explicitly; do not make an undeclared scratch pair a
second return value. Audit each actual body's scratch/clobber contract.

Acceptance: direct helper execution covers full unsigned range boundaries,
all signed quadrants, zero and overflow; low/high result bytes, preserved
registers, scratch guards and stack effects match the descriptors. No
cart/standalone result difference. Use independently maintained helper bodies
and preserve provenance; do not copy an old erroneous body and rename it.

### Slice 5: enable the shared contract atomically in actionc

Wire ordinary expressions and compound operations in both classic profiles
and MIR6502 to the typed selection/binding path. Route all relevant folds
through the shared evaluator. Enable under both runtimes together.

Earlier slices may use an internal development entry point for incomplete
paths, but no public legacy-arithmetic flag or Compatibility-only bug path is
introduced. Do not land a public state with corrected constants and knowingly
incompatible runtime operators as the completed rollout.

Acceptance:

- Every valid arithmetic case agrees across literals, CONSTs, runtime values,
  ordinary/compound assignments, arguments, returns and comparisons.
- Full-range CARD division and signed MOD match the independent oracle.
- Final narrowing does not narrow operands prematurely.
- Fault behavior is consistent, ordered and independent of runtime selection.
- Existing cartridge calls still work beside local arithmetic helpers; no
  unnecessary helpers are linked, and unsupported math overrides diagnose.
- Update conformance expectations only for documented semantic corrections;
  preserve original behavior in historical survey records, not modern oracles.

Record code-size/linkage changes separately from correctness. Existing
benchmarks may legitimately change size, but do not relax unrelated quality
budgets without investigating why.

### Slice 6: costed narrow signatures and shared divmod selection

Extend existing typed helper selection, byte-range/extension reasoning and
consumer/liveness analysis. Begin with measured u8/u8 and u16/u8 opportunities;
implement other asymmetric variants only when useful. Compatibility may keep
a less optimized selection policy but must use the same semantic contract.

For paired division and MOD, combine only operations on the same captured
typed values when dominance, ordering, faults, liveness and effects allow it.
Do not duplicate operand evaluation or substitute a reread across a call,
volatile access or aliasing store. Model both result definitions explicitly.
No source-name matcher or new general-purpose inliner is needed.

Acceptance: selection and rejection tests accompany execution oracles.
u16/u8 quotients above 255 and u8/u16 divisors above 255 remain correct.
Signed extensions, unknown ranges and high-dependent consumers retain the
general path. Cost includes staging, result adaptation and helper linkage;
report both selected cases and cases deliberately left generic.

### Slice 7: resume Oscar64 arithmetic ports, category by category

First translate `testsigned16div.c`: retain literal versus runtime divisors,
all nonzero coefficients -16..15, both signs, and guarded indexed outputs.
Its unrolling pragma must become actual literal expressions where needed;
simply deleting the pragma loses the axis under test.

Then translate `divmodtest.c`: retain quotient/remainder checks and extend
the independent oracle across CARD's high-bit range. Preserve original loops
where practical; clearly document any bounded host-grid adaptation. Keep
Action!-specific zero/overflow tests separate from valid C-domain translations.

Use the existing VM runner, compile once per mode/runtime, run fresh VMs,
check complete outputs/inputs/guards, and use watchdogs rather than timing
thresholds. Preserve expression structure when a failure appears. Count only
executed cases and distinguish semantic rejections from VM coverage.

Acceptance: all applicable six combinations pass. Keep port and compiler-fix
commits separate; update the source provenance and coverage tables.

## 5. Cross-target implementation guidance

68000's DIVS.W/DIVU.W distinguish signed and unsigned 32-by-16 division and
produce word quotient/remainder results. A backend can sign- or zero-extend
the source dividend appropriately, extract both results, and adapt the
overflow/zero behavior to the language contract. The INT_MIN/-1 wrapping
case must not silently inherit hardware overflow behavior.
See the [Motorola programmer's reference](https://www.nxp.com/docs/en/reference-manual/M68000PRM.pdf).

The 65816 instruction set has no native integer multiply/divide instruction.
It can implement the same operation with its own helpers, register widths,
direct-page/stack rules and fault delivery. Do not copy Atari zero-page ABI
assumptions into its IR. See the
[WDC datasheet, instruction table](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf).

Initially test typed lowering, signatures, effects, endian/result layout and
rejection of unsupported machine-code emission on these canaries. Add the
same executable conformance matrix when native emitters become available.

## 6. Validation and completion

The shared matrix includes:

- All BYTE/CHAR/INT/CARD ordered type pairs; literal/runtime combinations and
  explicit conversions, not invented signed BYTE semantics.
- Signed quadrants; 0, 1, -1; boundaries around 127/128, 255/256,
  32767/32768, 65535; exact and inexact quotients and divisors larger than
  dividends.
- INT_MIN/-1, static/converted/dynamic zero, dead-result fault preservation,
  guarded and unexecuted fault paths, and no post-fault source effects.
- Ordinary and compound operations; captured destinations; effectful operands;
  casts, arguments, returns, comparisons, indexed stores, and multiply/divide
  composition with the finalized multiplication types.
- Each implemented helper signature with direct ABI/effect tests, then
  public compiler execution through all profiles/runtimes.

Do not use agreement between backends or original ROM results as the oracle.
Use wider host mathematics and explicit source-width conversion rules. Record
historical divergences separately. Do not leave new ignored tests or encode
known wrong answers merely to keep the suite green.

After each code slice run focused tests and the required project checks:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml
```

Run the Oscar64 target after each port category:

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test oscar64_conformance
```

For arithmetic-emission changes, run/review the existing compatibility probe
sweep; document intentional byte differences instead of demanding historical
bug reproduction. Keep benchmark measurements separate from conformance.
Explain each snapshot change as semantic-contract, target-IR, printer-only,
or corrected output.

The first functional milestone is slices 1a through 5: one working arithmetic
contract with general correct helpers. Slice 6 is optional performance
generalization; slice 7 expands conformance coverage. Unsigned multiplication
result modernization, broader shift/addition policy, ROM fast-path reuse, REAL,
and the original-compiler VM mode remain explicit separate decisions/work,
not hidden requirements for that milestone.

## 7. Implementation record (2026-09-06)

### Delivered slices

1. Removed eager host division from constant-address probing; literal,
   converted and nested zero now produce semantic diagnostics. Fixed classic
   cached indirect/indexed memory facts after captured-place retargeting.
2. Added `semantic::integer::divmod`, shared by all relevant integer folders.
   Kept INT multiplication results deliberately. INT_MIN/-1 wraps; signed MOD
   follows the dividend, and CARD uses the full unsigned range.
3. SemIR and NIR normalize operand conversions before division, separately
   from the final destination conversion. NIR verifies the domain and preserves
   potentially faulting operations; classic expression-effect proofs and MIR
   dead-value/movement passes do likewise. 6502 uses signed/unsigned variants;
   68k/65816 canaries retain source width and signedness explicitly.
4. Added independently implemented compiler-owned unsigned/signed word
   kernels, selectively linked under both runtimes. Legacy division/remainder
   SET overrides cannot replace these operators. Existing cartridge services
   and the standalone SYSLIB remain available. Audited multiplication effects
   include the different cartridge and standalone scratch regions.
5. Enabled the contract in all six mode/runtime combinations. Owned helpers
   participate in runtime maps and `SET buffer=*` high-water calculations.
   Stopped destination-driven widening from changing division's operand tree.
   The signed port also exposed a Compatibility comparison staging bug:
   preparing a second computed indexed operand could overwrite the first;
   the existing stack-staging path now protects both materialized operands.
6. Generalized the existing helper selector to independent input widths,
   output adaptation and explicit additional result homes. Added unsigned
   byte/byte and word/byte helpers and a bounded captured-value div/mod fusion.
7. Added `testsigned16div.act` and `divmodtest.act` with independent VM oracles,
   without changing their arithmetic expressions to hide compiler failures.

### Private 6502 signatures and selection limits

All helpers clear decimal mode, may fault on zero, and clobber A/X/Y/flags.
Returning paths are stack-balanced; the scratch table describes only those
paths. Error has arbitrary memory/OS effects, and earlier source writes must
remain observable to its handler. These are private target signatures, not
changes to the Action! public calling convention.

| Operation | Inputs | Outputs | Writable scratch |
| --- | --- | --- | --- |
| Signed word quotient/remainder | A:X, `$84:$85` | A:X | `$82..$87`, `$C2..$C3` |
| Unsigned word quotient/remainder | A:X, `$84:$85` | A:X | `$82..$87` |
| Unsigned byte/byte quotient/remainder | A, X | A | `$82`, `$84`, `$86` |
| Unsigned word/byte quotient | A:X, `$84` | A:X | `$82:$83`, `$86` |
| Unsigned word/byte remainder | A:X, `$84` | A | `$82:$83`, `$86` |
| Signed/unsigned shared divmod | A:X, `$84:$85` | quotient A:X, remainder `$86:$87` | Same as corresponding word helper |

Byte results are explicitly zero-extended when their source result is a word.
A word/byte quotient is never truncated just because its divisor is a byte.
Unknown or wide divisors and signed operands retain the general path. The
existing byte proof recognizes constants, byte temps and zero high lanes;
immediately preceding unsigned extensions expose those lanes using the
existing use/definition index. It does not substitute pointer-cell rereads.

The initial speed/size cost charges actual generated helper bytes when not
already selected, per-call staging and output adaptation, and conservative
fixed-loop cycle estimates (eight cycles per code byte). These are an explicit
selection model, not measured benchmark timings or a globally optimal linker
decision. A pre-home consumer can conservatively charge linkage again; later
selection sees the helpers already selected. Classic uses the general kernels.

Shared-result selection is deliberately limited to adjacent same-domain MIR
operations on identical, uniquely defined captured values, with ignored output
carry and no input carry. Both result definitions are explicit and survive
home allocation. It does not cross stores, calls, loads or control-flow edges.
In particular, `q=x/y; r=x MOD y` currently reloads source storage and therefore
remains two operations. Broader fusion needs a reusable capture/alias proof,
not a source-expression matcher. This is a performance limit, not a semantic
or conformance gap.

### Permanent validation coverage

- Shared evaluator: exhaustive BYTE/CHAR pairs and independent signed/unsigned
  word boundary oracles, including zero and signed overflow.
- Emitted kernels: all 65,280 nonzero byte/byte pairs for both quotient and
  remainder, word/byte boundaries, full word boundary grids, and scratch guards.
- Direct MIR pairing: selection, reversed result order, both runtimes, emitted
  execution of both homes, and rejection of intervening effects, rereads,
  different operands/domains and carry dependencies.
- Public compiler: 2,688 VM type-pair cases (16 ordered types × 28 inputs × six
  modes/runtimes), plus multiplication composition, narrow-signature selection,
  fresh captures, dynamic-zero stops, discarded results and unexecuted paths.
- Frontend: nested/converted/compound/CONST/array-bound/fixed-address/return zero
  diagnostics and legacy override rejection. 68k and both 65816 canaries verify
  signedness and source widths; native execution is still out of scope.
- Oscar64: signed literal/runtime division adds 234 executions; the unsigned
  port retains all 1,190 original outer-loop values across six combinations,
  adding 7,140 executions. Combined Oscar64 coverage is 12,072 VM executions
  in 28 tests. The signed outer sweep is sampled (39 inputs); it is not claimed
  exhaustive. Original inner unsigned loops are retained.

Only the two MIR snapshots change (`div`/`mod` to `udiv`/`umod`). SemIR and NIR
snapshots are unchanged. Constant array bounds are lowered once, so inspecting
their typed values does not allocate extra evaluation-order IDs.
Their shared typed folding also preserves the existing zero result for shifts
of 16 or more, rather than accidentally masking the shift count to four bits.

The extension-to-lanes preparation is limited to unsigned division/remainder
consumers. Applying it to unrelated addition/index consumers exposed a
structural and execution regression in the existing widened-static-index tests;
that broader application was removed, and both original tests pass unchanged.

Compatibility capture check, without overwriting recorded artifacts: a clean
build of baseline `5777b0f` and the implementation were compared using the
existing probe catalog. Only `arith` changes from that baseline (309 to 565
load-file bytes), due to corrected locally linked arithmetic; its sweep policy
now records that intentional divergence. Existing original-cart differences
in `bool_edges`, `bools`, `control_flow`, `signedge`, and the already-accepted
cases are unchanged. `strnam` fails in both builds with the same
`fnam+$FFFF` relocation-overflow diagnostic. Consequently the historical sweep
is **not wholly green**; this is pre-existing drift, not newly passing coverage.

TURTLE's size guard retains its 1,078-byte application limit and separately
accounts for up to 175 owned-helper bytes (128-byte RemI, 47-byte RemU8).
This is an explicit linking cost, not a relaxation of the application budget.
No other quality budget was changed.

Final validation:

- Full root suite: **2,763 passed, 22 pre-existing ignored**. The final full
  library rerun passes all **2,426** library tests, including emitted shared
  divmod and malformed private-signature checks.
- Full isolated VM suite: **112 passed, none ignored**. This includes all
  **12,072 Oscar64 VM cases in 28 tests** and the six modern-arithmetic tests.
- Latest focused arithmetic VM matrix and full library tests were rerun after
  the final result-home effect and constant-folding checks.
- NIR snapshots and the **33-fixture NIR sweep** pass; the broad verified
  corpus contains **321 entrypoints plus five declared nonentrypoints**.
- `git diff --check` passes. Unrelated pre-existing workspace files were left
  untouched.
