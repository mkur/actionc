# Native 65816 direct acyclic word copies

Implemented on main in `84a265f`, qualified on 2026-09-22. This slice implements
direct scheduling for acyclic word edges from the
[remaining-copy inventory](MIR65816_COPY_INVENTORY.md). Cyclic edges retain full
staging; the inventory's broader selective-staging forecast remains future work.
Public ABI v1, image v3, o65 profile v1, stack guards and frame allocation remain.

## Selection contract

After complete existing word-edge preflight, disjoint destinations and whole-word
source overlaps define dependencies: a source must be read before another copy
overwrites its bytes. A stable topological schedule emits A16 LDA/STA directly,
preferring the original final assignment last. If dependencies force it earlier,
an extra LDA from its destination restores the original full A and N/Z. C/V and
all other preserved state remain unchanged. Every assignment, including a
self-copy, still loads and stores; no home is removed or coalesced.

A cycle or partial source/destination overlap selects the complete staged path.
Scheduling finishes before any copy is emitted. Mixed-width/unsupported edges
retain bytewise selection. Existing staging slots remain allocated and validated,
even when unused. No value lives in X/Y/DP, across a call or across a MIR boundary.
Only private edge-copy access order changes. See the
[emission contract](MIR65816_EMISSION_CONTRACT.md).

## Measured result

The [frozen baseline](benchmarks/65816-acyclic-edges/baseline.json), committed in
`1c8530b` before selection changed, predicts one selected corpus edge: optimized
`loop_rotation` initialization. Its three assignments no longer stage. All six
vectors execute it once, removing 18 staging store/reload pairs per incoming I
state across the corpus: 36 instructions, 180 cycles, 36 stack-byte reads and
36 writes. Static code shrinks by 12 bytes. Its cyclic backedge remains staged.

| Optimized `loop_rotation(13)` | Before | After |
| --- | ---: | ---: |
| Code bytes | 160 | 148 |
| Cycles | 1,146 | 1,116 |
| Instructions | 268 | 262 |
| Stack-byte reads | 181 | 175 |
| Stack-byte writes | 178 | 172 |
| Peak stack bytes | 26 | 26 |

All six rotation inputs match this result. The other 27 raw/optimized Action
builds are byte-identical, including `sum_loop(13)` at 146 bytes / 1,587 cycles
raw and 120 bytes / 1,212 cycles optimized. Frames, incoming displacements, guards,
DP traffic and existing fusion/single-copy/forwarding counts are unchanged.

The [exact delta](benchmarks/65816-acyclic-edges/delta.json) checks all 28 final
instruction streams, permitting only the predicted staging-pair removals,
interleaved retained loads/stores and required address remapping. It also checks
every measurement field. The new `acyclic_word_edges`, `acyclic_edge_words` and
`acyclic_word_edge_sites` metrics are separate from existing single-word metrics.
Typed multi-word identities validate actual dependency order and final bytes;
they do not drive CPU execution. Interior instruction suffixes cannot become
extra counted edges.

## Qualification

- 68 emitter/proof tests and 60 affected compiler integration tests pass. The
  existing emission boundary snapshot is unchanged; no NIR fixture changed.
- All 94 native tests pass in debug and release hosts, with 374 identical saved
  artifacts and 418 matching compiler/fixture input hashes. Coverage includes
  ordinary/fused branches, repeated sources, swaps/rotations, mutable parameters,
  mixed-width fallback, calls, aliasing, relocated o65, IRQ/NMI and stack faults.
- Independent ca65 sequences verify acyclic/reordered/self-copy cases, final A
  and all admitted status combinations, exact cycles and untouched staging bytes.
  Emitter tests check dependency direction, cycles, partial overlap, frame and
  fixup preservation. The decoder rejects unsafe orders and stale A restoration.
- The long-dispatch regression now uses 32 word arguments: direct scheduling made
  the former 16-word edge fit a short branch. Both forms still execute in banked
  images and at two o65 placements.
- Corpus builds match for LF/CRLF. Debug/release reports contain identical 264
  records, with both incoming I states checked for every vector. The known
  optimized vbcc `unlink` vector-0 failure remains explicit in both hosts.
- All 33 Python comparison tests pass, including exact transformation rejection.

The [qualification record](abi/action65816-acyclic-edges-qualification.json)
binds source, tools, manifests and artifacts. The
[measured snapshot](benchmarks/65816-acyclic-edges/after/tables.md) and
`target/acyclic-edges-after` are the new immutable quality baseline.

Reproduce the saved-artifact checks with:

```sh
python3 -B tools/compare65816/check_acyclic_edges.py \
  target/control-3c-after target/acyclic-edges-after \
  --baseline docs/benchmarks/65816-acyclic-edges/baseline.json \
  --output docs/benchmarks/65816-acyclic-edges/delta.json
python3 -B tools/compare65816/report.py --input target/acyclic-edges-after \
  --output docs/benchmarks/65816-acyclic-edges/after
```

For missing artifacts, reconstruct the baseline in an isolated `94421a3` checkout
and the new compiler/runner from `84a265f`, using the
[comparison workflow](../tools/compare65816/README.md). Preserve historical records.
Reconstructed runs have new provenance; save them in new directories instead of
overwriting the frozen baseline hashes.
Unused staging reservation removal is a separate slice: recheck actual writes,
alignment, parameter offsets, frame extent and guards before changing allocation.
