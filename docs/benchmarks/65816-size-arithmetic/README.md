# Native 24-bit arithmetic

Three-byte ADD/SUB and AND/OR/XOR now use an A16 low word followed by an
exact A8 high byte. They operate on checked stack homes and numeric constants,
without DP staging. ADD/SUB carries or borrows across the mode change.
Existing pointer-step selection and unsupported-operand fallbacks remain.

| Measurement | Before | After |
| --- | ---: | ---: |
| Stack ADD/SUB sequence, A16 entry | 33 bytes | 15 bytes |
| Same sequence, native CPU cycles | 59 | 32 |
| `MetadataBytes` routine | 171 bytes | 157 bytes |
| `MetadataBytes` frame / local stack peak | 12 / 22 bytes | 12 / 22 bytes |
| Existing generated Exec routine code, 960 routines | 492,302 bytes | 491,585 bytes |

The sequence comparison executes the previous bytewise form and the new form
from identical CPU/memory state and checks them against a ca65 reference.
For `MetadataBytes`, A is already eight bits at the addition: the arithmetic
changes from 31 to 17 bytes, including its required mode switches.

Exec saves **717 bytes** across 35 routines; no routine grows and no frame
changes. Its frame-map validator accepts all 960 maps. These are emitted-code
measurements from existing generated inputs, not booted-image qualification
or a container/padding size claim. Bank-zero reservation delta: **0 fixed
bytes, 0 bytes per task**.

Validation: 314 MIR65816 unit tests pass (one existing ignored test), 39
emission/o65/state-boundary integrations, three new size-arithmetic runtime
tests, five existing long-arithmetic runtime tests and four storage-demand
runtime tests. The new IRQ/NMI test interrupts both task domains across the
low-word operation and A8 tail. Tests cover raw/optimized NIR, carry/borrow
and 24-bit wrap, exact three-byte accesses, constants and bitwise operations,
mutable parameters, address differences, relocation, guards, and LF/CRLF
fixture input. Default-feature library compilation also passes.

Local override provenance: compiler HEAD
`ee4cc114f31287be1aa66a36923df7f30a19a866` plus this working-tree change;
compiler source-tree digest
`3dbf327bb50a387cef60b28b45bca1fbe87400a10f1eccdbad662f216dd13cd0`;
compiler executable SHA-256
`7928a361e199e835061184029329d778d7172bd330bd12052cbd77d96b03ca11`.
The Exec revision, 133 generated/module inputs, digest procedure and compile
command are unchanged from the
[storage-demand measurement](../../MIR65816_STORAGE_DEMAND_PLAN.md#measurements).
Exec's compiler pin and play image were not updated.
