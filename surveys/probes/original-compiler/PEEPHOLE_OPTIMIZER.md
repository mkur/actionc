# Original Peephole Optimizer Notes

These notes summarize observed original Action! code-shape optimizations from
the probe corpus, TN comparisons, and manual cross-checks. Some of these
patterns may be instruction-selection templates rather than a separate explicit
optimizer pass inside the cartridge. For `actionc`, it is safest to treat them
as local late-codegen compatibility rules.

## Working Model

The original compiler appears to use a small, machine-aware peephole layer. It
tracks very local register facts, especially known `Y` values, and combines
nearby 6502 instructions into compact forms when doing so is obviously safe. It
does not look like a global optimizer: the same broad source shape can still
reload a register in one nearby block while reusing it in another.

The most useful compatibility principle is:

- Match evidence-backed local patterns when they preserve semantics.
- Prefer helper-level implementation over one-off special cases.
- Do not intentionally reproduce original compiler bugs.
- Keep accepted divergences when `actionc` emits smaller or saner code and the
  difference is documented.

## Known Register Reuse

The original commonly keeps `Y=#1` or `Y=#0` live across short straight-line
sequences.

Observed examples:

- `booleq.act`: after loading `Y=#1`, the original emits multiple `STY`
  stores while `Y` remains known.
- `boolthen.act`: byte equality true arms can inherit a `Y=#1` hint and store
  directly with `STY`.
- `boolword.act`: word equality branches are shaped so the true arm can keep
  the known `Y=#1` value.
- `tn_value_index.act`: `actionc` is intentionally one byte smaller in one
  accepted divergence because it keeps `Y=0` live where the original reloads
  `LDY #0`.

This suggests local register tracking, not broad liveness analysis. Hints are
best invalidated at calls, machine blocks, uncertain branches, and any
instruction that changes the tracked register.

## Constant Store Walks

The original can reuse `Y` for adjacent or nearly adjacent byte constant
stores. The strongest focused probe is `ywalk.act`.

For:

```action
a=1
b=2
c=1
d=0
```

the original keeps `Y=#1` live across the intervening `A` store and walks down
to zero for the final store. It does not use an upward `INY` walk for storing
`2` in this probe; it uses `A` for that value.

Current rule of thumb:

- Reuse known `Y` for straight-line byte constant stores.
- Use conservative `Y` walks for tiny, evidence-backed cases such as `1 -> 0`.
- Probe before adding broader walks such as `1 -> 2`, because the original did
  not choose that form in the focused probe.

## Word And Pointer Byte Order

The original stores multi-byte values in little-endian memory, but it often
emits high-byte operations first because that cooperates with `Y=#1`.

Observed forms:

- `CARD ARRAY` stores often use `Y=1` for the high byte, then `DEY` or `Y=0`
  for the low byte.
- `CARD POINTER` stores and loads use high-byte-first instruction order.
- Record pointer field access follows the same pattern for multi-byte fields.
- Pointer-like ABI argument staging can load or store the high byte first while
  preserving the externally visible low-byte-first ABI.

This is an instruction-order peephole, not a layout difference. Memory remains
little-endian.

## Pointer And Word Increments

The original has compact increment-by-one forms for pointer and word-like
values.

Observed examples:

- `pointers.act`: `pointer ==+ 1` uses `INC low` with carry propagation to the
  high byte.
- `bool_edges.act`: word increment-by-one and compact word-zero branch shapes
  were needed for exact output.

These are good candidates for local expression/codegen helpers because they are
small, common, and semantically straightforward.

## Zero-Page Preference

The original takes advantage of user-forced zero-page declarations and direct
zero-page addressing.

Observed examples:

- Pre-code declarations such as `BYTE screen=$E6` reserve no load-file storage
  and compile to direct zero-page accesses.
- TN uses these aliases to force tighter and faster code.
- Dynamic byte indexes and temporary effective addresses use established
  zero-page scratch slots, especially `$AE/$AF`, with care not to overwrite a
  still-needed source pointer.

For compatibility, zero-page declarations are both a storage-layout feature and
an optimization hint. They should not be treated like ordinary globals.

## Branch And Boolean Shapes

The original prefers compact byte comparisons when possible.

Observed forms:

- Byte equality against a literal often lowers to `LDA`, `EOR #literal`, and a
  branch.
- Pointer dereference comparisons can use `EOR (zp),Y` or `CMP (zp),Y`
  directly instead of materializing through an extra temporary.
- Word equality/inequality tests branch out on high-byte mismatch and then use
  the low-byte result for the final branch.
- Signed comparisons use careful byte-shaped high/low tests rather than a
  generic helper for every case.

These patterns are compatibility-sensitive because they interact with nearby
known-register hints and branch layout.

## Control-Flow Tails

The original performs small tail-layout peepholes:

- A bodyless `PROC` can fall through to the next routine body. TN relies on
  this for an empty `PROC Error()` shape.
- A routine ending in an infinite `DO ... OD` with no direct `EXIT` does not
  need an extra implicit `RTS`.
- Machine-block tail routines and explicit machine-code blocks can suppress or
  change implicit `RTS` emission depending on shape.

These are more like layout peepholes than expression optimizations. They should
be kept conservative, because an incorrect omission of `RTS` is a real control
flow bug.

## Scratch-Pair Discipline

Several probes show that the original keeps independent pointer expressions in
distinct zero-page scratch pairs when both are live.

Observed examples:

- Pointer/array copy shapes use separate pairs such as `$AC/$AD` and `$AE/$AF`
  to avoid clobbering the source while preparing the destination.
- The pointer stress probe exposed that `actionc` must not reuse the same pair
  for both sides of an indexed pointer assignment.

This is not a size optimization by itself, but it is part of the original's
late code-shape discipline and prevents subtle miscompiles.

## Open Questions

Useful future probes:

- More systematic constant-store walks: `0 -> 1`, `1 -> 2`, `2 -> 1`, and
  longer adjacent runs.
- Register invalidation around calls, runtime helper calls, and machine blocks.
- Pointer arithmetic with non-constant offsets and mixed byte/card indexes.
- Final-loop tail handling with nested loops and `EXIT` in inner-only loops.
- Whether branch hints should be propagated to other comparison families beyond
  the currently matched equality and signed-edge probes.

Until those are probed, keep new peepholes narrow and source-shape guarded.
