# Late spill-pruning experiment — 2026-11-13

The address-stable experiment is implemented behind
`Mir6502Config::enable_late_spill_pruning`. It remains **off in both default and
optimized configurations**. It has not established a further TN size saving.
The [design addendum](../../MIR6502_LATE_CLEANUP_DESIGN.md#allocation-pruning-experiment)
and [implementation plan](../../MIR6502_LATE_CLEANUP_IMPLEMENTATION_PLAN.md#allocation-pruning-experiment-slices)
define the restricted contract.

## Measured results

| Case | Producer-enabled baseline | Pruning enabled | Additional saving |
| --- | ---: | ---: | ---: |
| TN, cart | 9,970 bytes | 9,970 bytes | **0** |
| TN, standalone | 10,077 bytes | 10,077 bytes | **0** |
| Independent pointer transform, cart/standalone | 99 bytes | 99 bytes | 0 |
| Argument capture under ZP pressure, cart | 118 bytes | 118 bytes | 0 |
| Argument capture under ZP pressure, standalone | 200 bytes | 200 bytes | 0 |
| Direct MIR control with an unused trailing byte | measured by executable oracle | one byte smaller | **1 byte** |

TN retains all six candidate spill declarations. The frame-address exposure
check excludes the two in `SetWin`; removing the four in `FormatDirectoryRow`
would move surviving storage, so the core allocation guard declines before
costing. TN remains at three materializations, 43 emission invocations, 258 inner
emission probes and 24,926 rewrite-analysis builds. Enabling the experiment adds
one diagnostic transaction, with no additional TN cost emissions or analysis
builds.

The pointer-transform control also has interior holes. Under ZP pressure, the
argument-capture control has an unused tail byte, but its machine-code ABI
descriptor exposes frame addresses. Both runtimes retain it before costing.
Unknown readers/writers also retain allocations unless existing callee summaries
provide exact footprints. No proof-only storage receives an invented physical
address.

The positive direct-MIR control establishes that the transaction can delete an
unused tail declaration and emit a smaller executable. Its byte-transform
oracle covers all 256 inputs, both runtimes and two origins. It is a constructed
compiler-boundary control, not a source-level optimization win. All complete
source-fixture comparisons in this experiment remain unchanged.

## Implementation and limits

The existing spill-pruning scan supplies candidates. Structured references,
terminators, inline-assembler references, historical frame exposure and explicit
memory regions constrain them. Opaque assembler names are interpreted only by
the assembler; the pruner conservatively retains the owning frame. An exact
declaration-only delta preserves live IDs, operations, ABI and zero-page mappings.

Two distinct certificates guard placement: surviving core planning-layout
addresses and storage attributes must remain equal; converged emission must
also retain the final physical placement of surviving objects and descriptor
backing data. The second check catches deferred arrays moving despite stable
planning addresses. Earlier late
proofs replay against the real candidate map. Missing placement or proof data
rejects the transaction. A common final-cost predicate requires a smaller
packaged image and no routine instruction growth. Any failure restores the
complete producer-cleaned baseline, preserving earlier savings.

This is not general frame compaction. Interior holes require retained core
allocation predicates that can be replayed after live objects move. A second
independent gap is that historical late proofs may still depend on storage whose
last executable use disappeared. Both need explicit proof contracts before
broadening pruning; repeatedly invoking the existing early pruner cannot supply
them.

## Reproduction and validation

The frozen baseline is `83236a1f`; the experiment implementation is `304025f3`.
The previous
[producer-cleanup receipt](../6502-late-producers/qualification.json) remains
unchanged. This experiment's receipt is [qualification.json](qualification.json).
Detailed outputs and logs are under ignored `build/mir6502-late-spills/`.

The API probe in [late_spill_pruning.rs](../../../tools/compare6502/late_spill_pruning.rs)
compares complete `CodegenOutput` values, trace on/off selection, packaged image
sizes and work counters for eight source/configuration cases, both runtimes and two
origins. It toggles only the pruning flag, with store/producer cleanup enabled.
The [Python harness](../../../tools/compare6502/late_spill_pruning.py) also checks
all 54 default CLI artifacts against the frozen producer-enabled hashes:

```sh
python3 tools/compare6502/late_spill_pruning.py \
  --compiler build/checked-arena/shared-target/debug/actionc-emit \
  --output build/mir6502-late-spills/qualification
```

To run the API probe separately, compile it against a matching `actionc` rlib and
pass a new artifact directory:

```sh
rustc --edition=2024 tools/compare6502/late_spill_pruning.rs \
  --extern actionc=/path/to/libactionc-HASH.rlib \
  -L dependency=/path/to/debug/deps -o /tmp/late-spill-probe
/tmp/late-spill-probe /tmp/late-spill-results
```

The executable byte oracle and MADS reconstruction/relocation test are in
`mir6502::late_cleanup::pruning::tests`. The source-pressure fixture also runs
through real LF/CRLF files and the full VM, checking public bytes, ordered writes
and A/X/Y/SP/status. Backend qualification commands and counts are recorded in
the receipt. No shared frontend or NIR contracts changed.

Checks passed:

- 1,314 MIR6502 unit tests; 14 optional checks ignored by the normal run.
- 131 integration tests across all 28 `mir6502_*` targets and `cli_emit_modes`;
  11 optional checks ignored.
- 31 VM tests across 13 affected targets; one frozen allocator benchmark ignored.
- The explicit pruning MADS test: eight exact reconstruction/relocation
  comparisons across cart/standalone, original/shifted origins and LF/CRLF.
- The positive executable byte oracle: 2,048 executions across all byte inputs,
  pruning off/on, both runtimes and both origins; exact one-byte image savings.
- 54 frozen default CLI artifacts and 32 complete API comparisons, including
  traced/untraced equivalence and bounded work.

The new tests do not change shared frontend/NIR contracts. The ignored checks
and actual backend commands are listed in the receipt; this local qualification
does not replace full CI.
