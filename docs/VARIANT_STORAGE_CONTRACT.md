# Variant storage and checked transfers

Modern Action! variant objects must not partially overlap independent live
objects. This replaces the earlier guarantee that typed variant assignment
could relocate an object onto part of its source.

## Allowed storage relationships

- Distinct variables, array elements and fixed-arena slots have separate extents.
- Multiple pointers may refer to the same object. Exact self-assignment is valid
  but still checks that the source is constructed and has valid active tags.
- A nested variant is a subobject of its parent. Its containment is intentional,
  not two independent objects sharing bytes.
- Alternatives inside one variant still share the canonical payload storage.

For a whole-value assignment involving inline variants, the complete source and
destination ranges must be identical or disjoint. This includes a record with
inline variants: its complete checked value is the transfer unit. Pointer fields
are not followed. Plain records and untagged unions without inline variants keep
their existing overlap-safe assignment semantics.

Overlapping relocation/compaction remains a low-level byte operation, outside
typed variant assignment. Such a move must preserve the representation and
update/invalidate old pointers; it does not leave two independent live objects.
There is no new relocation API or automatic object-lifetime tracking.

## Enforcement and ordering

The compiler captures the destination address first, then evaluates and captures
the source address. A direct value-to-value assignment checks the complete ranges
when their relationship is not established by canonical declaration/field facts.
Different starts less than the value extent apart are rejected. Equal starts and
exact adjacency are allowed. Address comparisons/subtraction use the target's
unsigned ADDRESS width, not Action!'s 16-bit CARD. Subtracting the smaller address
from the larger avoids wrapping end-address calculations.

A partial-overlap failure uses the nonreturning InvalidVariantOverlap fault:
Error(106) on Atari, in classic/MIR6502 and both runtimes. Invalid active tags
separately report InvalidVariantTag/Error(105). The guard runs before
any destination copy. Active source tags are then validated in place, including
all active inline nested values, before any destination write. If Error returns,
the existing defensive stop remains. Native fault emission still requires a
native Error adapter; this change does not add one.

The address-evaluation expressions may have effects before a failure. Those
effects retain source order; a failed transfer is not a rollback transaction.
No new compile-time pointer-flow analysis or global live-object overlap registry
is introduced. Fabricating overlapping views, accessing dead/unallocated memory,
wrapping an object around the address space or corrupting it through raw bytes
does not become safe merely because a particular tag happens to be valid.

## Lowering and simplifications

SemIR owns this contract. It lowers checks through ordinary typed comparisons,
branches and the existing fault operation. NIR/MIR do not infer source types or
receive blanket no-alias promises. `CopyBytes` retains its overlap-safe contract
for every consumer, including ordinary records and unions.

Once both addresses are captured, in-place validation performs no user calls
except a terminal fault. The checked assignment therefore transfers the source
once, without an extra whole-value RHS snapshot. Its logical copy count falls
from two to one, and the hidden aggregate capture disappears. A statically known
self-copy validates but transfers no bytes. Direct compiler-owned objects and
canonical fields do not require a runtime overlap guard.

CASE selector captures, pattern snapshots, ordered argument captures and
constructor preparation remain: they protect values across intervening effects.
Removing those needs separate lifetime/effect proofs. Constructor initialization
is refined independently: active fields are written/copied first, only the
remaining target-layout gaps are zeroed, and the tag is written last in the
private capture. Full active aggregate extents include their copied padding.
No pre-clear is emitted when payload and tag cover the complete value. Larger
unused regions retain compact range loops; singleton gaps need only a store.
Static declaration zero images remain load-time data on Atari.

The source-order regression also exposed a classic wide-stride pointer selector
that evaluated an index before loading its base and used repeated additions.
It now uses the existing captured-base/constant-scale fallback. This is a shared
address-selection repair, not a variant-specific backend path.

Regression coverage lives in `tests/variant_storage_contract.rs` and
`tools/vm-runtime-tests/tests/variants.rs`: identical/disjoint/adjacent ranges,
both overlap directions, page-crossing extents, source-order address effects,
nested invalid tags, unchanged destination bytes on failure, target-width NIR
and unchanged plain-record/union copy semantics.

Storage-contract baseline acceptance: all 2,962 compiler tests and 195 pinned VM tests passed, together with
NIR snapshots, the 44-file NIR and 167-file MIR6502 sweeps, and
`cargo check --all-targets`. That change did not alter existing IR fixtures.
The subsequent constructor-initialization refinement updates the generic,
nested-pattern, guarded-CASE and variant-match NIR snapshots intentionally:
redundant full-value clearing and its pointer/counter locals disappear. Focused
coverage in both `tests/variant_initialization.rs` suites checks native padding,
complete active field extents, poisoned temporary bytes, page-crossing gaps,
single writes and tag-last ordering.
