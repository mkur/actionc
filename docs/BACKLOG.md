# Backlog

This file tracks cross-cutting compiler work that does not naturally belong to a
single backend or survey note.

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

Two issues were observed while prototyping the dedicated runtime error codes;
neither is needed by the final machine-only Error adapters:

- In the resident `CartLongError(BYTE code)` prototype, an `ASM OPAQUE`
  block loading/pushing `code`, followed by the Action call
  `CartLongErrorEntry(code,0,code)`, failed materialization with
  `call-arg-producer: replacement effects do not match the declared delta`.
  Minimize the call-argument rewrite/effect case before changing its proof.
- An inline `JSR CartLongErrorEntry` referring to the resident declaration
  `PROC CartLongErrorEntry=$04CB(BYTE code,x,y)` linked to an emitted empty
  parameter-capture stub rather than `$04CB`, falling through into the next
  routine. Audit absolute-procedure relocations during resident linking.
  The final Atari adapter explicitly calls `$04CB`; standalone retains its
  symbolic relocation to the linked SYSLIB Error body.

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
