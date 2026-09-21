# Native 65816 empty-edge cleanup

Completed on 2026-09-21. Empty MIR edges now restore A16 only when necessary
and emit their existing typed JML. Selection, focused execution and the recorded
baseline are committed as `3dd5cb2`. The
[emission contract](MIR65816_EMISSION_CONTRACT.md) defines the invariant; the
[qualification record](abi/action65816-empty-edges-qualification.json) binds the
results to source, tools and saved machine-code artifacts.

## Scope and invariants

The emitter resolves the successor and verifies argument count before selecting
an empty edge. A missing target label is diagnosed before changing code or mode
knowledge. No argument or destination copy is skipped on a nonempty edge.

| Local A-width knowledge | Emission | Saving per edge |
| --- | --- | --- |
| A16 | JML | 4 bytes, 6 cycles |
| A8 | REP #$20; JML | Unchanged |
| Unknown | REP #$20; JML | 2 bytes, 3 cycles |

Local labels still invalidate mode knowledge. In particular, a conditional
true-edge label retains REP; the false edge already knows A16. Goto and
Fallthrough use the same checked empty-edge path. Jumps are retained, including
to adjacent blocks and equal branch targets. Jump threading, fallthrough
elimination, branch relaxation and global width propagation remain outside
this change.

ABI v1, image v3, the o65 profile, frame allocation, stack guards, and nonempty
word/byte copies are unchanged. A/X/Y, S, D, DBR, I and arithmetic flags survive
empty transfers, with M restored to zero. There are no data-memory reads or
writes, DP accesses, pushes or new helpers. Exec816's compiler pin is unchanged.

## Measurements

The immutable baseline is the [word-edge snapshot](benchmarks/65816-word-edges/after/tables.md),
qualified at `932a0cf`. All 224 saved file hashes across 56 builds and all 264
matching debug/release records were checked before changing selection.
Four representative forecasts were recorded in
[baseline.json](benchmarks/65816-empty-edges/baseline.json).

The [new snapshot](benchmarks/65816-empty-edges/after/tables.md) and
[complete delta](benchmarks/65816-empty-edges/delta.md) retain the unchanged
14-pair / 66-vector corpus. Measurements include guards and RTL:

| Kernel / input / mode | Bytes before → after | Cycles before → after | Unchanged stack depth |
| --- | ---: | ---: | ---: |
| maximum(13,41), optimized | 116 → 110 | 117 → 111 | 6 |
| sum loop(13), optimized | 160 → 154 | 1,780 → 1,735 | 16 |
| loop rotation(13), optimized | 192 → 186 | 1,344 → 1,314 | 26 |
| byte sum($12FFFC,16), optimized | 258 → 252 | 4,700 → 4,646 | 22 |

All four forecasts match exactly. The strict delta requires identical stack
reads/writes, argument layouts, frame maps, guard costs and observed stack
peaks. The [additional check](../tools/compare65816/check_empty_edges.py)
compares every Action instruction stream to the old stream with only the
redundant empty-edge SEP/REP instructions removed and JSL/JML addresses
relocated. It also checks identical DP traffic and fusion/word-copy counts,
and exactly three saved cycles per removed executed instruction. Unaffected
emitted files are byte-identical.

Both host comparison commands still report the existing optimized vbcc unlink
vector-0 failure in both incoming I states. Complete measurements are saved,
debug/release results agree and all vbcc records are identical to baseline.
No failure exemption is applied.

## Validation

Three new native tests cover Goto, Fallthrough, ordinary and fused branches,
both branch arms, both compiler modes and I states, a preceding A8 store,
independent ca65 encodings, exact transfer instruction boundaries and cycles,
register/flag preservation, no data traffic, and malformed decoder inputs.
Unit tests check known A8/A16/unknown mode knowledge, exact bytes, typed fixups,
invalid targets/arity, nonmutating errors and unchanged nonempty fallbacks.
The comparison decoder recognizes both compact empty-edge forms while retaining
byte and word-copy decoding and relocated target checks.

Full native qualification passes **74 tests in debug and release**, with **232
identical saved artifacts**. Compiler checks pass 34 native library tests and
58 ABI/emission/o65/CLI integration tests. Existing decoder/delta Python tests
pass 5/6 cases. Corpus LF/CRLF builds are identical.

General IRQ coverage reaches 2,245 raw and 2,087 optimized enabled instruction
addresses. Targeted coverage retains six fused windows and all 24 post-CMP
truth/task combinations across 300 task/PC sites per mode, with all 222 word-copy
sites retained. Full register/frame restoration and both seeded IRQ/NMI schedules
pass. The smaller site counts reflect removed instructions. Relocated o65
probes require the compact false/true empty-edge forms at both code placements;
existing nonempty-edge, mixed-ABI, guard and context probes remain green.

This is an emission-only change. No semantic or NIR contract changed, so the
full root suite and repository-wide NIR sweep were outside the validation scope.
Qualification executes serialized machine code on the corrected pinned VM;
board and Exec816 loader acceptance remain separate integration work.
