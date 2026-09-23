# Backlog

This file tracks deferred compiler work, including cross-cutting issues and
backend follow-ups linked to their design and measurement documents.

## Native 65816 Code-Size Reduction

Status: signed word comparisons and direct branch fusion are implemented.
Other items remain backlogged at user request. Prioritize
emitted code size, with execution cycles as a secondary constraint.
This ordering supersedes the earlier throughput-first recommendation in the
[Dijkstra comparison](benchmarks/65816-dijkstra/README.md) and the remaining
allocation work in the [65816 quality plan](MIR65816_CODE_QUALITY_PLAN.md).

The subsequent Exec816 audit's user-selected **BYTE and native pointer
comparisons** are complete. The
[implementation plan](MIR65816_BYTE_POINTER_COMPARISONS_PLAN.md) delivered native
BYTE predicates, three-byte Eq/Ne/null tests, compact Boolean materialization
and adjacent branch consumption. Frozen optimized Exec shell code shrinks by
72,039 bytes (12.5%); optimized Dijkstra shrinks by 418 bytes to 5,779, with
frames and guards retained. See the
[measured results](benchmarks/65816-byte-pointer-comparisons/README.md).
The other candidates below remain deferred; the table below retains its
original baseline rather than silently replacing historical evidence.

The optimized Dijkstra baseline from `d84a27eb` contains 6,197 Action code bytes
versus 1,477 for vbcc, a 4.20× gap. The difference affects ordinary routines:

| Routine | actionc bytes | vbcc bytes |
| --- | ---: | ---: |
| Find | 2,132 | 470 |
| Enqueue | 836 | 241 |
| Main | 1,267 | 202 |

The 22 stack-check sequences occupy 990 bytes, or 16% of the Action module.
Their low dynamic cost (0.151% of benchmark cycles) does not make them cheap in
code size. Excluding those bytes only for accounting leaves 5,207 bytes, still
3.53× vbcc's entire module; guard removal is not proposed.

The [signed word comparison plan](MIR65816_SIGNED_WORD_COMPARISONS_PLAN.md)
is implemented. Its eight Dijkstra branch sites reduce raw code 6,365→5,878
bytes and optimized code 5,779→5,290 bytes, with unchanged homes and all 22
guards. Optimized `Find` is now 1,887 bytes. The small corpus and frozen Exec
shell remain byte-identical. See the [results](benchmarks/65816-signed-word-comparisons/README.md).
The historical table above remains the original baseline.

Remaining candidates, each requiring a focused implementation plan when resumed:

The [current-source Exec audit](benchmarks/65816-exec-size-detail/README.md)
measures Exec `c3500c8` with guards enabled on pin `2d73c03a` and main `9f16e08b`.
Main already saves 73,505 optimized executable bytes; pin integration still
requires reconciling the stack-check layout field and Exec qualification.
The audit recommends outgoing-padding-only initialization as the next small
slice: 13,756 bytes of payload zero stores are immediately overwritten by
argument writes. It also inventories compact guard branches, native 32-bit
Eq/Ne, direct BYTE constant returns and unused final index shifts. These remain
unimplemented; their forecasts overlap where they replace the same code.
The user-selected [outgoing argument padding plan](MIR65816_CALL_PADDING_PLAN.md)
is now proposed; it preserves ABI v1, all guards and the existing payload-copy
loop while removing only duplicate payload clears. Implementation is pending.
The older candidates below retain their original measurements.

1. **Broader local branch relaxation.** Extend the existing
   [layout finalizer](../src/mir65816/emit/layout.rs) beyond its selected dispatch
   sites to eligible local conditional transfers and unconditional jumps.
   The original optimized Dijkstra inventory found 67 non-guard six-byte
   conditional sequences whose destinations fit two-byte branches in the
   then-current layout: 268 bytes of encoding opportunity. Re-inventory current
   output before planning; signed selection changed those sites. This is an inventory,
   not an implemented or qualified saving; overlapping jump savings must not
   be counted twice. Preserve bank/range checks, labels, fixups, PER continuations,
   proof metadata and o65 relocation.
2. **Compact address construction.** Reduce repeated pointer materialization,
   temporary copies and bytewise scaled-index expansion through ordinary
   lowering. Use native-width operations where justified while retaining full
   24-bit results and bank carries. Calls, helper clobbers, aliasing and volatile
   effects remain barriers unless existing facts prove a narrower effect.
3. **Compact stack-guard encoding.** Retain every required check, its position
   before stack mutation and its failure behavior. Reduce encoding overhead
   without changing the public ABI, stack bounds or preemption guarantees.

Prefer improvements to basic selection and the existing checked emission/layout
machinery. Broader register allocation and ABI changes remain separate work.
For each resumed slice, report actual raw/optimized byte reductions by routine
and whole program across both the small corpus and Dijkstra; track cycles and
stack/DP traffic as secondary checks. Preserve correctness, ABI and guards, and
validate final machine bytes, relocation and relevant IRQ/NMI behavior through
the affected 65816 tests. Keep the original comparison immutable; its equivalent
driver simplification for unsupported DIV/MOD remains part of that baseline.

## Archived TN Compatibility Return Diagnostic

The optional `surveys/tn/check-stability.sh` currently stops on unchanged
`corpora/tn/original/extracted/SRC/TN.ACT.atascii:336`: BYTE Fnamecmp's
`RETURN(-1)` is rejected as returning INT. Reproduced with `--profile compat`
during the directory migration. Investigate the compatibility typing rule with
a focused original-language regression; do not edit the archived sample to
make the diagnostic disappear. The maintained modern builds pass their
separate behavioral tests.

## Classic Indirect Expressions in TN Directory Code

Status: found during the TN directory migration; compiler fixes not started.
The [directory model tests](../tools/vm-runtime-tests/tests/tn_directory_model.rs)
exercise source forms that avoid these issues; keep independent failing
reproducers when addressing the compiler, rather than adding TN special cases.

- A CARD comparison such as `ordinal>=tags.capacity`, where tags is a record
  pointer, can lose the high operand while preparing the indirect RHS field.
  The listing loads the operand high byte, overwrites A to form the field
  address, then continues the comparison. A local CARD snapshot of capacity
  avoids this; include both operand orders and fields past offset 255 in the fix.
- `p^==&(mask!$FF)` emits AND mask followed by EOR $FF in classic, changing the
  meaning of the parenthesized RHS. Computing the inverted mask separately
  avoids this. Test compound assignments with nested RHS operators against
  ordinary assignments in both backends.
- A CARD parameter named start in NextTagged was emitted as the address of a
  later PROC Start when assigned to a local ordinal. A distinct parameter name
  avoids the collision. Resolve through semantic storage identity and cover
  variable/parameter names shadowing routine names.
- Rendering `output(n)=Internal(batch.summary(n))` can overwrite its destination
  address in $AE/$AF while evaluating the record-pointer source, then write into
  the source instead. Capturing the converted byte in a local before the store
  avoids it. Cover computed lvalues, indirect field/array reads and argument
  evaluation effects together; the callee's preserves annotation alone does
  not describe argument evaluation.

- After an indirect CARD equality, classic can replace a following constant
  `LDA #1` with `TYA` although the comparison has advanced Y to 7. The integrated
  one-file tag-all regression exposed this (`allIntent` became 7). Snapshotting
  compared fields avoids this; verify register facts across comparison branches.

## Classic Record-Copy Scratch Placement with SET

Status: backlogged at user request; implementation has not started.

Found while introducing PanelState in [TOMS Navigator](../samples/tn/README.md).
Whole-record assignment causes classic code generation to prepend copy scratch
storage before the source's legacy allocation `SET`s. Restoring the source
cursor then points into already emitted storage. This reproducer fails with
`--mode optimized --runtime cart`, reporting `compatible code pointer $2C00 is
before current output $2C0A`; its MIR6502 cartridge build compiles:

```action
ORG $2C00
SET $E=$E6
SET $F=0
BYTE POINTER screen
CARD POINTER allocp
SET $E=$2C00
SET $491=$2C00
TYPE State=[BYTE a,b,c,d,e,f CARD g]
State first,second
PROC Main()
  first=second
RETURN
```

- Fix the interaction between classic projection's generated copy scratch and
  source-controlled storage placement. Preserve `ORG`/explicit CLI origin
  precedence, zero-page pointer homes and the final `SET BUFFER=*` boundary.
  Do not special-case TN or require source offsets for hidden compiler storage.
- Add focused compilation and VM regressions for both modern backends and both
  runtimes. Verify scratch/code/data do not overlap, full record copies preserve
  evaluation order and alias/overlap semantics, and deferred storage remains
  outside emitted code.
- Once fixed, replace TN's two `MovePage(..., SIZEOF(PanelState))` calls with
  ordinary record assignments. Rerun its panel-transition, dispatch and storage
  checks in both backends, and measure load size and workspace changes.

## LONG Codegen Audit and Measured Optimization

Status: backlogged at user request; implementation has not started.

Follow up on the completed
[classic LONG support](CLASSIC_LONG_INTEGER_IMPLEMENTATION_PLAN.md).

- Measure emitted bytes and execution cycles for rotations, arithmetic,
  comparisons and calls across Compatibility, Optimized classic and MIR6502,
  with both cartridge and standalone runtimes.
- Use the existing LONG and Oscar64 rotation oracles as correctness baselines.
  Record reproducible measurements before choosing an optimization slice.
- First candidates in Optimized classic are constant shifts, increments and
  redundant four-byte copies. Implement general codegen improvements with
  before/after size and cycle results.
- Preserve typed intermediate widths, signedness, call order, volatile access
  counts, captured addresses and terminal faults. Retain the existing source
  expressions and independent expected results in regression coverage.

## Broader IF/CASE Expression Result Types

Status: backlogged at user request; implementation has not started.

The completed [IF/CASE expression implementation](Action_2027/IF_CASE_EXPRESSIONS_IMPLEMENTATION_PLAN.md)
supports integer and enum results. Arms already accept arithmetic, calls, casts
and nested selections; the remaining restriction is the result type.

- Extend result support in small slices: pointers first, including declared
  callable signatures; REAL; then records, unions and variants.
- Support selecting and returning a constructed variant value, for example:

  ```action
  LET value=IF ready THEN MaybeByte.SOME(n) ELSE MaybeByte.NONE FI
  ```

- Define compatible arm types and conversions in SemIR, preserving nominal
  aggregate identity, pointer/callable facts and existing aggregate value-copy
  rules. Carry the required facts through verified NIR.
- Preserve evaluation of only the selected arm, destination and argument
  capture, nested calls, volatile accesses and aggregate validation/fault order.
- Cover modern classic and MIR6502 with both runtimes. Keep explicit
  unsupported diagnostics for result types until their slice is complete;
  statement blocks that yield a value remain separate work.

## Oscar64 Conformance Regressions

The [first eight test ports](../fixtures/runtime/oscar64/README.md#compiler-regressions)
now pass all 258 VM cases (14 tests, no ignored cases). Both
`OSCAR-CLASSIC-WORD-INDEX` and `OSCAR-MIR-SELF-INDEX-STORE` are fixed without
changing the source loops or expected values. MIR post-home rewrites check
replacement dependencies, and verification rejects undefined private scratch
reads. The broader indexed-backing coverage audit remains a separate follow-up.

The [second-batch plan](OSCAR64_TEST_PORTING_PLAN.md) has arithmetic composition
and reverse-copy ports implemented. `OSCAR-CLASSIC-COMPUTED-INDEX` is now fixed:
the general pointer-index fallback retains its captured base across recursive
index materialization. All 2,172 Oscar64 VM cases pass, including the 120
formerly failing classic reverse-copy cases, with unchanged fixture expressions
and oracles. See [the diagnosis and fix](bugs/CLASSIC_COMPUTED_POINTER_INDEX_BUG.md).

Stage 3 adds 1,536 passing nested-call VM cases. `OSCAR-COMPAT-NESTED-CALL`
is fixed by sharing protected argument staging across classic profiles, looking
through casts, and materializing each stacked argument at the public ABI base.
All 3,708 Oscar64 cases now pass without changing port expressions or oracles.
The separately exposed optimized word-return accumulator-fact regression is
also fixed: inferred return facts now use the existing register/value equality
proof instead of equating `Unknown` descriptions. Focused execution covers
assignment, argument and pointer-index consumers, plus multiple return paths.
See the [diagnosis and repairs](bugs/CLASSIC_NESTED_CALL_ARGUMENT_BUG.md).

Stage 4 now distinguishes cartridge-compatible branches from the agreed modern
comparison-value extension. Shared comparison machinery supports BYTE 0/1
values in modern classic and MIR6502; Compatibility rejects value uses during
semantic analysis. The broader grid also exposed and repaired signed-subtract
overflow in both classic profiles. The 408 branch/count cases and 264 modern
value cases brought Oscar64 coverage to 4,380 cases in 24 tests; see
[the contract and repairs](bugs/COMPARISON_VALUE_MATERIALIZATION_GAPS.md).

Stage 5 is complete after [embedded fixed-length record arrays](EMBEDDED_RECORD_ARRAYS_IMPLEMENTATION_PLAN.md)
were enabled in modern profiles. The original record structures remain intact:
198 record-array copy cases run in all modes, and 120 inline-member cases run
in both modern backends, all with both runtimes. Total: 4,698 VM cases in 26
tests. The [newly exposed record-array gaps](bugs/RECORD_ARRAY_PORTING_GAPS.md)
and [scalar-copy pointer overlap](bugs/CLASSIC_INDIRECT_SCALAR_COPY_POINTER_BUG.md)
are repaired. The subsequent arithmetic rollout adds 7,374 executions (12,072
total in 28 Oscar64 tests). Volatile categories remain follow-ups;
wider-stride MIR copy fusion needs a nonconflicting scratch plan
before it can replace the current safe staged path.

## Oscar64 Volatile Test Port

Status: deferred at user request; not the next active test batch.

- Port `autotest/volatiletest.c` when this work is resumed, following the
  [deferred porting batch](OSCAR64_TEST_PORTING_PLAN.md#deferred-batch).
- Start with read/write ordering, unused reads, reads through small functions,
  and loop reads whose results are only partly used. Keep the DMA case as a
  separate follow-up slice.
- Use observable access counts or a deterministic VM test device; final RAM
  contents alone cannot detect removed, duplicated, or reordered accesses.
- Exercise all supported compiler profiles and both runtime link modes. Keep
  any compiler repairs separate from the test ports.

## Standalone Runtime Licensing

- Replace the GPL-only standalone `SYS` implementation with an independently
  maintained syslib under a more permissive license.
- Preserve the public `SYS` interface and selective-linking boundary so existing
  programs do not need source changes.
- Until that replacement exists, retain the standalone GPL warning for selected
  `SYS` procedures, compiler helpers, and their runtime dependencies.

## Standalone Runtime Error Diagnostics

Priority: low; deferred.

Standalone SYSLIB `Error` currently jumps through DOSVEC without printing a
message. See the [runtime error audit](ATARI_RUNTIME_ERRORS.md).

- Enhance the existing standalone `Error` implementation to open GR.0, print
  `Error: <code>` (for example, `Error: 101` for division by zero), and terminate
  through DOSVEC. Do not introduce a separate fatal-error API.
- Preserve the cartridge-compatible A/X/Y interface, reporting the code from Y;
  leave the cartridge `$04CB` binding unchanged.
- Keep selective linking and the arithmetic non-return guard if the handler
  returns. Avoid recursive error reporting if screen setup or output fails.
- Add VM coverage for visible output from a graphics screen, DOS handoff, and
  the absence of post-fault source effects in classic and MIR6502 builds.

## Debug-only Aggregate Overlap Guards

Agreed follow-up, separate from the runtime error-code split:

- Retain the variant-containing assignment non-overlap contract, exact aliases
  and ordinary record/union copy semantics.
- Diagnose provable violations at compile time; make uncertain partial-overlap
  guards debug/checked-build instrumentation rather than unconditional code.
- Keep check policy independent of language profile, backend and optimization.
- Do not implicitly change tag validation, division-by-zero or checked parsing.
  Error 106 remains the diagnostic for a detected forbidden overlap.

## Modern Integer Arithmetic

### MIR error-wrapper integration probes

Both recorded issues are repaired:

- `ASM OPAQUE` followed by `CartLongErrorEntry(code,0,code)` now materializes.
  Transaction validation counts every call-operand memory read, including
  repeated byte/word sources and indirect targets. Per-operation alias
  footprints remain deduplicated; the unchanged-effect check still rejects
  missing/extra reads and changed call effects.
- Symbolic inline-assembly calls to fixed-address procedures now use NIR's
  routine placement facts during MIR lowering. They retain their absolute
  addresses through resident selection/rebasing; relocatable routines retain
  stable IDs. Full addresses, low/high bytes and addends remain correct when
  the load origin changes.

Coverage includes `tests/inline_asm.rs`, MIR rewrite/standalone unit tests and
`tools/vm-runtime-tests/tests/runtime_error_wrappers.rs`. The VM probes check
the Error ABI and terminal return guard in classic/raw/optimized MIR with both
runtimes. The production machine-only Error adapters retain their existing
implementation.

### Arithmetic acceptance

The [legacy division/remainder audit](bugs/LEGACY_INTEGER_ARITHMETIC_AUDIT.md)
is complete. It found unsigned folding of signed expressions, signed runtime
helpers for CARD, the original remainder-workspace corruption, a compiler
panic on nested constant division by zero, an independent optimized-classic
captured-reload bug, and arithmetic-domain loss in target IRs. These audited
gaps are now repaired; the earlier port suites did not cover those domains.

Delivered by the [implementation plan](MODERN_INTEGER_ARITHMETIC_IMPLEMENTATION_PLAN.md):

- Repaired the host panic and captured-operand fact lifetime independently.
- Established one typed arithmetic contract for every actionc profile, including
  Compatibility, with consistent folding and runtime execution.
- Kept cartridge-linked programs: correct compiler-owned
  arithmetic beside existing cartridge services, also under standalone linking.
- Generalized existing helper selection to per-input/per-result signatures,
  explicit effects and costed specialization; preserve domains for 68k/65816.
- Added signed division and full-range CARD division/remainder ports after
  the coordinated semantic rollout. Both pass independent six-way VM oracles.

The bounded shared-divmod selector only combines adjacent operations on
identical captured MIR values. Repeated source expressions that reload storage
remain separate; broader pairing needs reusable capture/alias proofs. Narrow
helper selection uses a documented speed/size model, not benchmark-specific
rules. Native 68k/65816 executable arithmetic remains future backend work.

Historical behavior belongs to the separately planned original-compiler VM
mode, not to a bug-emulation branch in actionc. Multiplication result-type
modernization and mixed-type shift/overflow policies remain explicit language
decisions; the plan does not silently change them.

## Builtin Symbol Coverage

- Add tests that enumerate all valid Action! builtin symbols and verify that
  each compiler path recognizes them consistently.
- Cover semantic analysis, legacy/compat codegen, modern/MIR6502 codegen, and
  SemIR/NIR lowering where applicable.
- Distinguish intentionally unresolved symbols from missing support, so names
  such as resident variables and library/runtime routines do not silently drift
  between backends.
- Include builtin routines, predefined/resident variables, byte arrays, pointer
  forms, and aliases/case variants accepted by Action!.
