# TN Stability Check

TN's maintained sources on `main` are restored to the pre-directory-refactor
checkpoint `8ccdc683`. The newer batch/paging implementation is preserved on
`tn-ng` at `4baaac89`. Directory migration, bitmap, size-reduction and later
profiling reports below describe that newer source. Run their batch-specific
measurement scripts and profiling commands from `tn-ng`; their recorded results
are historical and do not describe the restored sample. Compiler improvements
and independent regression reproducers remain on `main`.

This check keeps TOMS Navigator in the compatibility workflow as a large real
program. The small original-compiler probes catch narrow codegen shapes; TN
catches broad layout regressions, include handling, and routine-local storage
changes that only become visible in a substantial source file.

The original MIR6502 listing baseline and optimization backlog are in
[`mir6502-optimization-opportunities.md`](mir6502-optimization-opportunities.md).
The [source modernization audit](source-modernization-audit-2026-09-15.md)
identifies uses for current language features, preservation constraints and
compiler limitations, with fresh builds of TN and TNDBG.
The [bitmap traversal audit](bitmap-scans/README.md) compares per-bit and
byte-wise scans, qualifies a smaller tail-clear loop and identifies the next
MIR6502 arithmetic and captured-bound proof work.
The [current code-size audit](size-audit-2026-10-13/README.md) attributes qualified
TN/TNDBG bytes, measures inlining/unrolling controls and ranks record-address
retention and intermediate value flow as the next size directions.
The [record base reuse plan](record-base-reuse-plan.md) stages the shared
address/alias proofs, reuse across reads and proven writes, bounded parameter
origins and final size/behavior qualification.
The [directory handling implementation plan](directory-handling-implementation-plan.md)
replaces the proposed row overlays with 64-entry batches, independent tagging
and a MyDOS-first migration toward FujiNet. The implementation and its VM checks
are preserved on `tn-ng`; see that branch's `samples/tn/README.md` for its memory measurements and acceptance notes.
The proposed [size reduction implementation plan](size-reduction-implementation-plan.md)
preserves the FujiNet paging contract while addressing shared entry addressing,
classic correctness, MIR6502 record/pointer generation and validation ownership.
The fresh final-output audit after carry and dual-pointer selection is in
[`mir6502-final-listing-audit-2026-07-24.md`](mir6502-final-listing-audit-2026-07-24.md).
The latest clean-head listing reanalysis and ranked MIR6502 backlog are in
[`mir6502-listing-reanalysis-2026-07-22.md`](mir6502-listing-reanalysis-2026-07-22.md).
An independent, artifact-pinned review of a separately supplied listing is in
[`mir6502-independent-listing-review-2026-07-25.md`](mir6502-independent-listing-review-2026-07-25.md).
The earlier caller-shadow analysis is retained as a historical measurement,
with its incorrect ABI premise marked in
[`mir6502-final-listing-analysis.md`](mir6502-final-listing-analysis.md). The
corrected contract, implementation slices, and measured removal result are in
[`mir6502-public-abi-shadow-correction-plan.md`](mir6502-public-abi-shadow-correction-plan.md).
The implementation roadmap for avoiding transient temp and internal scalar
storage is in
[`mir6502-home-elision-plan.md`](mir6502-home-elision-plan.md).
The follow-on plan for reducing costly MIR lanes before home planning is in
[`mir6502-residual-lane-reduction-plan.md`](mir6502-residual-lane-reduction-plan.md).

Run from the `actionc` repo root:

```sh
surveys/tn/check-stability.sh
```

The script compiles TN with `compat` from the archived original extraction and
with `modern` from the maintained source under `samples/tn/modern`, compares each
load-file size against the original Action! compiler `TN.COM`, and fails when a
profile exceeds its size budget.

The restored modern budget is +/- 1,792 bytes against the original 12,127-byte
file. The compatibility budget remains 512 bytes. The archived-source check has
an existing diagnostic: BYTE Fnamecmp rejects `RETURN(-1)` at line 336. Use the
[focused sample checks](../../samples/tn/README.md) to validate maintained TN.

Default inputs:

```text
compat source: corpora/tn/original/extracted/SRC/TN.ACT.atascii
modern source: samples/tn/modern/TN.ACT
baseline:      corpora/tn/original/extracted/TN.COM
```

The original Action! cartridge compiler symbol-table capture for the same
archived ATASCII sources lives in
`surveys/tn/original-symbols/`. It includes the raw VM JSON dumps,
flat TSV views for grep-friendly audits, and the exact VM command used to
produce them.

Use a local source or a fresh VM compiler output when investigating a specific
regression:

```sh
surveys/tn/check-stability.sh \
  --modern-source samples/tn/modern/TN.ACT \
  --original target/TN-selftest-suppressed.COM \
  --keep
```

This is intentionally separate from
`surveys/probes/original-compiler/sweep.sh`: the probe sweep is byte-exact
and fast; TN is a larger stability sentinel whose failure usually means “look at
layout or broad codegen drift first.”
