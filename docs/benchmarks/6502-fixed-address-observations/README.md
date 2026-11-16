# Fixed-address observations: TN qualification

2026-11-16. Implements the
[plan](../../MIR6502_FIXED_ADDRESS_OBSERVATION_IMPLEMENTATION_PLAN.md) under the
[unexposed-object contract](../../MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md).

## Result

TN remains **9,838 bytes** and its XEX is **byte-for-byte identical** to the
`8fd0f0f8` [DATA baseline](../6502-labeled-data/README.md), using optimized
MIR6502, ActionCart and origin `$3000`. TN/LIB source hashes and configuration
match the baseline. All 95 routine code/home footprints, all 34 leaf candidates'
entry/exit/caller facts, accepted trial costs and recorded work counts are
unchanged. Ord, Items, FindItem and Copy retain their previous placement savings;
Fnamecmp and Next retain their contracts.

Block/m7's observation scan now completes. Its symbolic global g9 (`screen`)
retains its identity and resolves to `$E6`; the indirect high-byte fetch is
recorded separately at `$E7`. This establishes the pointer-cell observations,
not the memory accessed through the pointer. Public pointee summaries remain
unchanged. The scanner and physical collector share fixed-global/alias resolution;
generated scratch accesses keep their separate residence checks.

## Remaining blockers

Sort still has **250 code bytes**, three mapped inline-home bytes and no ZP
homes. It passes native-body admission but is rejected before a placement
proposal. The first unresolved observation is now **_Cio, r21/bb41, m11**:
`JMP (ErrorHandler)`. ErrorHandler is ordinary global g41, so its physical address
and NMOS JMP high-byte fetch depend on final layout. Resolving declared fixed
storage cannot discharge that obligation.

The diagnostic collector now records every executable machine block's explicit
storage observations, with scope clearly separated from complete memory-effect
summaries. The selected TN program has four unresolved blocks:

| Owner | Machine block | MIR block |
| --- | --- | --- |
| _Cio | m11 | bb41 |
| _StrNam | m13 | bb43 |
| Range | m34 | bb71 |
| MakeJmp | m29 | bb424 |

These remain conservative and are outside this correction. No extra candidate
trial, layout-dependent shortcut or exception for TN was added. Sort has no
new placement/cost result to report.

## Focused validation

- Four new `fixed_observation_` unit tests pass: numeric/typed/alias equivalence,
  retained identities, checked offsets, unresolved and invalid references,
  ZP and NMOS JMP wrap boundaries, selector/bounds checks, full-ZP indexed-indirect
  observations, unknown pointees and both fetched bytes' overlap protection.
- Four existing `object_storage_` unit controls and the exact selected-input,
  executable and physical-map rollback control pass.
- Both pipeline controls pass with the extended fixed-pointer fixture. They
  check typed storage relocation input, actual resident-home selection in one
  trial, exclusion of declared ZP bytes, LF/CRLF image identity, and independent
  fallback when an opaque block remains.
- The existing VM group passes with placement off/on in ActionCart and
  Standalone. It executes the fixed-pointer ASM reader against two pointees,
  checks its output and unchanged pointer/neighbor bytes, then checks the native
  swaps, call inputs, returned values, page crossing, repeated calls and stack.
- One TN collection and diagnostic emission replay pass. The static leaf audit
  finds no uncovered direct accesses in Exact summaries, unresolved Exact
  identities or contradicted X/Y preservation claims.

The full TN dispatch VM target was not repeated because the complete image and
its placement are identical to the already executed DATA baseline. No full suite,
NIR sweep or historical compiler rebuild ran. The
[previously reproduced shared-check failures](../6502-labeled-data/final/validation.json)
remain a separate task; this qualification makes no repository-wide green claim.

## Artifacts and work

One traced source-to-executable generation took **20.599 seconds**, sampling
**540,528 KiB** resident memory. Rust compilation and diagnostic replay are
excluded. This is neither peak RSS nor a repeated performance comparison.
Materializations remain 6, emission invocations 145, inner emission probes 870,
preservation replays 17, source-storage inventory builds 60 and cross-call
placement trials 1. All recorded work counters match the baseline.

The XEX SHA-256 remains
`a3b369ace0a492a1b1e0032c03d8da7b86cde09b0a4a2c5e0595db8777673f2f`.
Raw image, full MIR, map, relocations and assembly are retained under
`build/fixed-address-observations/final/`. The probe hook was removed.

- [Comparison, decisions and observation records](final/comparison.json).
- [Compiler facts, machine observations and leaf assembly](final/facts.compact.json).
- [Instruction audit](final/audit.json), [control assembly](final/controls.asm)
  and [control MIR](final/controls.mir).
- [Compiler/source receipt and artifact hashes](final/receipt.json).
