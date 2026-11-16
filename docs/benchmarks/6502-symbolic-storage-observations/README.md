# Symbolic storage observations: TN qualification

2026-11-16. Implements the
[plan](../../MIR6502_SYMBOLIC_STORAGE_OBSERVATION_IMPLEMENTATION_PLAN.md) under the
[unexposed-object contract](../../MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md).

## Result

TN is **9,824 bytes**, **14 bytes smaller** than the `96e91785`
[fixed-address baseline](../6502-fixed-address-observations/README.md), using
optimized MIR6502, ActionCart and origin `$3000`. Source hashes and configuration
match. All 95 routines remain, and all explicit machine observations now resolve.

| Routine | Code before | Code after | Inline homes before / after | Saving |
| --- | ---: | ---: | ---: | ---: |
| Sort | 250 | 246 | 3 / 1 | 6 bytes |
| MoveMenuBar | 117 | 115 | 4 / 4 | 2 bytes |
| Draw | 147 | 145 | 1 / 1 | 2 bytes |
| Tag | 90 | 88 | 2 / 2 | 2 bytes |
| TagAll | 89 | 87 | 0 / 0 | 2 bytes |

Sort's `h` now occupies `$E2/$E3`; its one-byte `spill96` home remains inline.
The same cross-call proposal saves 86 bytes rather than 80, with Sort accounting
for the additional six. Ord, Items, FindItem and Copy retain their previous
savings; Instr, Fnamecmp and Strcpy retain their earlier selections.

The other eight bytes come from four dead `store.b zp1, a` staging stores.
With explicit observations complete, existing terminal cleanup can remove each
store while keeping the following A-to-X transfer. The image is already eight
bytes smaller before parameter placement. No new store-elimination rule or
per-routine size exception was added. See the
[selected MIR differences](final/selected-changes.diff).

Accepted image sizes are now `10039 -> 9947 -> 9910 -> 9824`, compared with
`10047 -> 9955 -> 9918 -> 9838`. Final complete-image acceptance and rollback
remain authoritative; no routine grew in this qualification.

## Resolved observations

- **Range:** SArgs capture is one typed ABI operation, referencing its first
  parameter and five-byte contiguous frame. Its descriptor is no longer scanned
  as instructions. Helper effects remain conservative.
- **_Cio:** the ordinary global pointer cell in `JMP (ErrorHandler)` resolves
  against final layout, including the NMOS high-byte fetch rule.
- **_StrNam / MakeJmp:** signed indexed bases remain expressions until layout.
  The scanner records the full byte-index extent; it infers no X/Y bounds from
  the source object's size. Exact callee-summary checks remain unchanged.
- **Block:** its previously resolved fixed `$E6/$E7` fetches remain intact.

All 34 leaf entry, analysis and rewrite summaries are unchanged. The four caller
fact differences are instruction-index shifts after deleted stores, with the
same residence-check outcomes. Fnamecmp and Next retain their contracts. The
static instruction audit finds no uncovered direct accesses or unresolved
identities in Exact summaries, and no contradicted X/Y preservation claims.

## Validation

Focused checks pass: typed-capture verifier and byte/relocation equivalence at
two origins; SArgs ABI, linkage, override and listing controls; actual LF/CRLF
listing/fixture paths; symbolic/fixed observations; old/new layout witnesses;
missing-map rejection; cleanup observations; and exact rollback.

Three object-storage pipeline controls pass. The positive case selects ZP homes
with ordinary symbolic jumps and signed global/local operands present. Opaque
code keeps independent callers eligible. An explicit `$A0/$A1` dispatcher
conflicts with a proposed `$A1` resident home, rejects the combined trial, and
returns a byte-identical baseline. The positive dispatcher uses a declared `$E9`
pointer cell so its physical obligations are disjoint from selected homes.

The VM controls pass in ActionCart and Standalone. SArgs captures all five mixed
argument bytes and returns past its descriptor with a balanced stack. The
object-storage group runs with placement off/on, exercising fixed and symbolic
pointer cells, indexed reads/writes, a dispatch table, array swaps, call inputs,
results, repeated calls and declared sentinels.

Broad qualification exposed an additional ABI liveness obligation: a wide call
can use `$AC/$AD` as argument bytes 13–14 even when its caller uses those bytes
as pointer scratch elsewhere. Once opaque descriptor observations were removed,
terminal cleanup incorrectly deleted twelve such stores in ADPCM decoding.
Disabling cleanup restored the independent C-reference result. Post-home home
liveness and definition-use analysis now read populated call homes, including
paired and absolute ZP lanes; pre-materialization effects are unchanged. The
grouped regression also proves that unrelated and later unused stores remain
dead. The ADPCM control checks the resulting emitted program, not just its size.

The full affected-backend run is recorded in
[validation.json](final/validation.json). Five optimizer unit tests, five
integration tests and four VM tests also fail on the baseline; the suite is not
green. The unit failures match the
[previous baseline evidence](../6502-labeled-data/final/validation.json), and
the integration/VM failures were reproduced on `96e91785`. Their expectations
were not weakened. After the ABI correction, all 102 selected integration tests
and 13 VM tests pass, including ADPCM, cross-call controls and TN dispatch.
The repeated MIR6502 unit run has 1,437 passes and the same five baseline failures.
External-assembler tests and the optional C-reference feature were not enabled.

## Artifacts and work

Materializations remain 6, emission invocations 145, internal emission probes
870, preservation replays 17, source-storage inventory builds 60 and cross-call
trials 1. Rewrite-analysis builds increase from 47,614 to 47,646. No additional
candidate compilation or subset retry was introduced.

The final traced generation took **20.544 seconds**, sampling **531,632 KiB** resident
memory. Rust compilation and diagnostic replay are excluded. This is not peak
RSS or a repeated performance comparison.

The final XEX SHA-256 is `76496fc2c4415ba09e613584e5f0ef7f3a0978a0132f6efb7e07aa9208650804`.
Full MIR, image, maps, relocations and assembly remain under
`build/symbolic-storage-observations/final/`. The diagnostic hook was removed.

- [Size, facts, observations and decision comparison](final/comparison.json).
- [Compiler facts and leaf assembly](final/facts.compact.json).
- [Static instruction audit](final/audit.json).
- [Control assembly](final/controls.asm) and [MIR](final/controls.mir).
- [Compiler/source receipt and artifact hashes](final/receipt.json).
