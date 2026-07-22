# TN Stability Check

This check keeps TOMS Navigator in the compatibility workflow as a large real
program. The small original-compiler probes catch narrow codegen shapes; TN
catches broad layout regressions, include handling, and routine-local storage
changes that only become visible in a substantial source file.

The original MIR6502 listing baseline and optimization backlog are in
[`mir6502-optimization-opportunities.md`](mir6502-optimization-opportunities.md).
The latest clean-head listing reanalysis and ranked MIR6502 backlog are in
[`mir6502-listing-reanalysis-2026-07-22.md`](mir6502-listing-reanalysis-2026-07-22.md).
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
with `modern` from the maintained source under `../modern`, compares each
load-file size against the original Action! compiler `TN.COM`, and fails when a
profile exceeds its size budget.

The modern budget is wider because the maintained TN source is newer than the
archived 1.22 source used by the original baseline and intentionally uses a
different optimized layout. Its default is currently +/- 1792 bytes. After
modern routine-entry trampoline elision, scaled `(zp),Y` word-index lowering,
straight-line propagation, and internal parameter-storage elision, the accepted
load-size delta is -1682 bytes (10445 generated versus 12127 in the original
baseline).

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
