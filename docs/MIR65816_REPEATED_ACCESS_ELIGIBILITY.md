# Eligibility for repeated three-byte memory accesses

Slice 9 of the [pointer micro-optimization plan](MIR65816_POINTER_MICRO_OPTIMIZATIONS_PLAN.md)
is an eligibility audit. Its result is **no new external-access admissions**.
Existing verified facts do not establish all the properties needed to repeat
an indirect or external middle-byte access. Slices 10 and 11 remain deferred;
this document does not grant permission to the selector.

## Required proof

Two words at offsets zero and one stay within a three-byte object, but the
middle byte is accessed twice. Before selecting that sequence, the compiler
must establish all of the following for the particular operation:

- A stable address and a complete three-byte extent. Each addressing mode must
  preserve the intended bank carry and remain within its displacement bounds.
- Ordinary memory whose reads have no device or other observable side effects.
  Repeated stores also need permission to perform an additional write.
- No intervening alias, interrupt, task or device action that can make the
  repeated access observe or overwrite a different value. A non-atomic pointer
  type does not itself grant this permission.
- A source/destination schedule that preserves the complete value and the
  address base. Repeating a private byte does not justify repeating an external
  one, and a load proof does not automatically establish a store proof.
- A verifier-backed fact attached to stable storage/region identities and a
  defined validity interval. Unknown provenance, merged pointers without the
  same proof, calls and machine operations must conservatively invalidate it.

For example, an interrupt changing the middle byte between the two word loads
can change the result relative to the existing low-word-plus-bank load. Between
two word stores, the repeated middle-byte store can overwrite an interrupt's
update. Neither sequence is an atomic three-byte access.

## Audit of current facts

| Existing fact | What it establishes | What it does not establish |
|---|---|---|
| `NirObjectLayout` | Object size and alignment | Device behavior, concurrency or permission to repeat an access |
| `NirStorageBackingClass::Ordinary` | Ordinary allocated backing rather than an absolute/alias declaration | Exclusive access through every derived pointer |
| `NirStorageFacts::is_proven_private_to_invocation()` | Unescaped invocation storage under the existing conservative address-use proof | Privacy of memory addressed by a pointer stored in that object |
| `NirMemoryRegion` and `NirMemoryEffects` | Conservative read/write footprints | Stability between accesses or an idempotent-write guarantee |
| `volatile: false` on MIR loads/stores | Eligibility for ordinary access-width selection | A complete repeated-access proof |
| `Mir65816AddressBase::Indirect` | A captured address value | The referenced allocation, its extent, device semantics or asynchronous ownership |
| ABI stack/DP homes | Captured values in invocation/domain-owned storage | General permission for accesses through an arbitrary pointer |

These facts live in [NIR storage analysis](../src/nir/analysis/storage.rs),
[NIR types/effects](../src/nir/ir.rs), [MIR65816](../src/mir65816/mod.rs) and
[place lowering](../src/mir65816/lower.rs). Lowering a dereference retains its
address value; it does not invent referent provenance or exclusivity.

The current [emission contract](MIR65816_EMISSION_CONTRACT.md) therefore remains:
ordinary external/indirect three-byte transfers use a low word and a bank byte;
volatile accesses retain ascending individual bytes. Overlapping words remain
restricted to the already admitted captured-home transfers. Null reductions,
casts and edge copies operate on those captured values, not their pointees.

## Follow-up boundary

An external-access implementation needs a separate shared-contract slice.
SemIR must own any new source-level promise; verified NIR must retain its typed
storage/region identity, access permissions and validity. MIR65816 may then
consume that evidence to choose an encoding. No executable source strings,
routine-name exceptions, new default concurrency assumptions or benchmark-only
admissions are acceptable.

A future implementation must test both proof production and rejection: escaped
objects, arbitrary pointers, MMIO, volatile accesses, aliases, invalid extents,
barriers, joins and asynchronous updates. Loads and stores need separate
admission tests and runtime traces, including bank crossings and unchanged
fallback traces. Shared-contract changes require NIR snapshots, the fixture
sweep and the full compiler tests.

Slice 12 can proceed independently if it preserves the existing external
accesses: capturing the low word in X until the bank byte has been read can
permit a dying private address home to be reused. Such a change needs its own
bounded allocation/selection proof; it does not satisfy or bypass this gate.
