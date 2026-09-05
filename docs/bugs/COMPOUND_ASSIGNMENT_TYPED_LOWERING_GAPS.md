# Compound assignment loses the operation's type

Status: fixed by embedded-record-array slice 4c, 2026-09-05.
These ordinary-language defects do not require experimental record arrays.
Public embedded-array syntax remains disabled pending slice 5.

## Reproducers and repair

- `BYTE ARRAY items(2)` followed by `items(1)==*3` previously failed NIR
  verification: multiplication must produce cartridge-compatible INT.
- With `items(1)=13`, `items(1)==/Divisor()` and a CARD-valued divisor of 256,
  MIR previously returned $FF instead of zero by truncating the divisor.

SemIR now reuses ordinary arithmetic-result typing and carries the final store
conversion separately. NIR computes INT multiplication or CARD division, then
casts the result to BYTE. Its multiplication verifier remains strict. Bare
array compounds update their canonical pointer cells rather than computing
with the element type. Classic consumes projected operation facts and shares
one captured-place path across ordinary arrays, pointers and embedded fields.
The same fallback fixes indirect word shifts that reused destination scratch.

The two known-bug characterizations are now correct-oracle execution tests.
Semantic tests cover all BYTE/CHAR/INT/CARD operand pairs and ten operators.
Experimental execution tests cover effectful field indexes and same-target
RHS writes. The public VM fixture covers both runtimes and all three modes,
with guards, signed values, widening/narrowing and counted RHS calls.

## Cartridge ordering evidence

The new probe `surveys/probes/original-compiler/compound_order.act` was compiled
and run with the original Action! 3.6 cartridge in actionc-vm. It writes:

| Result address | Value | Meaning |
| --- | --- | --- |
| $0600 | 42 | Scalar RHS changes the target from 3 to 40, then returns 2. |
| $0601 | 42 | The same operation through a named array element. |
| $0602 | 42 | Pointer destination captured before RHS retargets the pointer. |
| $0603 | 10 | The second buffer remains unchanged. |

Integer compound order is therefore: capture the address once, evaluate the
RHS, load from that captured address, compute, convert, store. Classic's new
captured path initially read the old value too early; NIR already ordered the
integer load after the RHS. Native REAL's separate lowering is unchanged.

To reproduce, drive the sibling VM's original-compiler profile with
`action-q-input` and `action-headless-getkey`, register this source as
`H:COMPOUND.ACT`, and send monitor commands `C "H:COMPOUND.ACT"` then `R`.
The standard monitor injection points are $A2E0 and $B2F5, as used by the VM's
`scripts/run-probe`. Stop on input idle and dump $0600–$0603.

## Arithmetic scope and code quality

The companion `compound_arithmetic.act` probe confirms a cartridge quirk:
for INT -513 and 256, ordinary division returns -2, but ordinary and compound
MOD both return 2, not C's -1. This repair does not redefine cartridge arithmetic
helpers. The VM matrix uses nonnegative MOD operands and logical RSH oracles.

Correct typing exposes a high byte that some BYTE stores discard. The existing
MIR discarded-high-product proof was generalized for Add/Sub/And/Or/Xor with a
sole adjacent truncation consumer. It retains memory reads and rejects live
high lanes, carry contracts and high-dependent operations such as division.
WARPDEM's existing code-size limit is retained, not relaxed.

Compatibility language restrictions and the public embedded-array gate are
unchanged. Aggregate initializers, subobject relocations and full record-copy
validation remain slice 5; no Oscar64 stage-5 cases are claimed here.
