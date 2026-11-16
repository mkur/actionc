# Labeled DATA: TN qualification

2026-11-16. Implements the [plan](../../LABELED_DATA_BLOCK_IMPLEMENTATION_PLAN.md).

## Result

Optimized MIR6502 TN, ActionCart at `$3000`, remains **9,838 bytes** against
the recorded `996c8842` [baseline](../6502-unexposed-object-storage/README.md).
Both TN and TNDBG now declare their six menu tables with `DATA name = [...]`.
The tables occupy the same **204 bytes**, including string self-addresses and
character `$9A` markers. Each emitted payload matches the baseline after
normalizing only those self-address relocations; every pointer is also checked
against its actual target address. No descriptor, terminator or `RTS` is added.

The six labels disappear from the routine inventory (101 → 95 routines,
40 → 34 leaf candidates). Every remaining routine has unchanged code bytes,
inline-home bytes and ZP-home bytes. DATA uses typed mutable static storage;
it never enters the executable machine-block scanner.

| Trial | Before | After | Saving |
| --- | ---: | ---: | ---: |
| Parameter homes | 10,047 | 9,955 | 92 |
| Argument handoff | 9,955 | 9,918 | 37 |
| Cross-call homes | 9,918 | 9,838 | 80 |

These trial boundaries and accepted routines are identical to the baseline.
The migration adds no size saving, and placement adds no further saving.
Ord, Items, FindItem and Copy retain their accepted cross-call changes.

## Sort's remaining blocker

Sort still has **250 code bytes**, three mapped inline-home bytes and no ZP
homes. It passes native-body admission, but discovery rejects it before a
proposal with `unresolved explicit observation`. There is no Sort placement or
cost result to report. Its Fnamecmp call contract is unchanged.

The new diagnostic identifies the first unresolved observation as **Block,
routine r16, block bb36, machine block m7**. This is executable assembly in
`LIB.ACT`, including `sta (screen),y` and `lda (screen),y`. The operand retains
global identity g9; that global has absolute address `$E6`. The observation
scanner's indirect-address path currently resolves only directly numeric memory
operands, so it cannot establish the pointer's two physical fetches from this
symbolic global. DATA removes the menu-table ambiguity but does not fix that
separate scanner limitation. Other unresolved blocks may remain after it.

The next scanner correction can resolve explicitly fixed global addresses while
retaining identity and correct page-wrap behavior. This qualification makes no
change to that proof or to conservative handling of unclassified raw code.

## Evidence and work

One completed traced generation measured **19.942 seconds**, sampling
**538,400 KiB** resident memory. This excludes Rust compilation and diagnostic
emission replay; it is neither peak RSS nor a repeated performance benchmark.
An earlier qualification attempt stopped during parsing and produced no image;
it exposed the contextual `data` operand regression covered by the final tests.

Materializations (6), emission invocations (145), internal emission probes (870),
preservation replays (17), source-storage inventory builds (60), and the single
cross-call trial are unchanged. Shared rewrite analysis builds fall from 50,356
to 47,614 after removing the six pseudo-routines.

The final XEX SHA-256 is
`a3b369ace0a492a1b1e0032c03d8da7b86cde09b0a4a2c5e0595db8777673f2f`.
Layout replay reproduced its bytes and relocations exactly. The temporary probe
hook was removed. Raw image, map, relocations and full selected MIR are retained
under `build/labeled-data/final/`; versioned evidence includes:

- [Comparison and exact table checks](final/comparison.json).
- [Compiler facts and leaf assembly](final/facts.compact.json) and
  [instruction audit](final/audit.json).
- [Control assembly](final/controls.asm) and [control MIR](final/controls.mir).
- [Source/compiler receipt and artifact hashes](final/receipt.json).

The static audit finds no uncovered direct accesses in Exact summaries,
unresolved Exact identities or contradicted X/Y preservation claims. It is an
audit aid, not an execution proof.

## Validation limits

Focused frontend, exact-emission, physical-observation, rollback and VM controls
pass, as do the 52-fixture NIR sweep, feature inventory and feature floors. TN's
menu/dispatch/panel VM test passes for TN and TNDBG in classic Optimized and
MIR6502 modes with ActionCart. DATA fixture checks exercise LF/CRLF through
parsing, lowering and printing. MyDOS disk operations are outside these VM tests.

Full shared validation is **not green**. The library run reports 3,449 passes,
five optimizer failures and 16 ignored tests, stopping before the full integration
suite. The snapshot check fails on the existing `enum_values` enum identity
(expected SymbolId(82), actual SymbolId(83)). All six failures reproduce with
identical assertions on clean pre-DATA commit `996c8842`; see the
[validation record](final/validation.json). The plan records the check scopes.
