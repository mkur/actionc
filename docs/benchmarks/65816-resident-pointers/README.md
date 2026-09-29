# Resident pointer identities

Compiler baseline `50c89af5`; implementation `996247d0` and `dab630a6`.
The [implementation plan](../../MIR65816_RESIDENT_POINTER_LEAF_PLAN.md) extends
private-pointer promotion and the existing three-slot DP allocator. Typed casts
and zero-offset address identities share a complete pointer value and lifetime.

| Measurement | Previous / raw stack path | Resident pointers |
| --- | ---: | ---: |
| AddHead code | 146 bytes | 96 bytes |
| AddHead frame / local peak | 4 / 4 bytes | 0 / 0 bytes |
| AddHead temporary spills | 0 bytes | 0 bytes |
| Entry-through-RTL VM cycles | 277 | 195 |
| Exec routine code, 960 routines | 472,114 bytes | 472,064 bytes |

The [generated assembly](addhead.asm) matches the independent handwritten
reference's 96 bytes and 195 cycles. The compiler assigns item to `$80`, chain
to `$83`, and first to `$86`; these values remain resident through all four
stores. There is no local stack frame, entry reservation guard or cleanup.
Raw NIR retains its previous 146-byte stack implementation.

The full Exec comparison uses exactly the same 133 generated source/layout
inputs as the [local-load measurement](../65816-local-loads/README.md).
Only AddHead changes: **50 bytes saved**, one smaller frame, no growth. All
960 final frame maps pass Exec's validator. The [routine table](exec-routines.csv)
includes unchanged routines; [provenance](provenance.json) records compiler,
source, input and image hashes plus the qualified CPU/tool versions.
Counts exclude data, alignment and container overhead.

Reproduce the Exec build using the command in the
[address-consumer measurement](../65816-address-consumers/README.md), changing
the output to `/tmp/exec816-resident-pointers.a816.json`. The listing uses the
existing decoder with an in-memory version-3 envelope; version-4 arithmetic
fault metadata does not change instruction bytes.

Validation covers raw and optimized compilation, exact external access traces,
bank crossings, overlapping nodes, the empty-list sentinel, fixed/o65 placements,
LF/CRLF source and assembly, alias-group pressure/fallback, private storage
legality, stack bounds, register state and context suspension. The dedicated
AddHead test injects IRQ at every reachable enabled instruction in both task
domains: 140 raw and 96 optimized `(task, PC)` sites, with NMI and dispatcher
reentry. The independent reference measures only entry through RTL, excluding
caller argument setup and teardown.

All **46 native cases** pass in debug across focused batches and in one release
qualification run. The targets are `resident_pointers`, `pointer_allocation`,
`pointer_reload`, `pointer_preemption`, `pointer_values`, `pointer_coalescing`,
`pointer_forwarding`, `address_consumers`, `home_demand`, `stack_checks` and
`stack_allocation`. Run them through `tools/native65816-runtime-tests/qualify.py`
with explicit `--test` arguments; add `--release` for the release profile.
The committed fixture and tests regenerate the reference and execution evidence.

The backend unit batch plus focused fixture follow-up passes 337 cases (one
existing ignored); 28 emission and seven promotion integration cases pass.
NIR snapshots and the 51-fixture sweep pass. The required full repository run
leaves two unrelated failures: `nir_corpus` hard-codes 362 successes while the
unchanged corpus has 363, and `samples` cannot resolve `SHARED.SCREEN` from the
pre-existing untracked `samples/vbxe/shared/lines.act`. Neither is suppressed.
Older fixtures assuming every cast has a stack home or a leaf still contains
empty-frame guards were updated to check current allocation and coverage.

Reserved bank-zero delta: **0 fixed bytes / 0 bytes per task**, including guards,
alignment and unused capacity. These nine bytes already belong to compiler DP
scratch. Task stack reservations and interrupt headroom are unchanged.
This is a compile-only local override for Exec; the compiler pin and play image
remain unchanged. Compiler/native checks do not qualify the hosted system.
