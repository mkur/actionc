# Action! 65816 native physical ABI v1

Status: **specified; scalar emission and context bridge exercised; full kernel-subset qualification pending**.
ABI identity: `action65816.native.v1`. Target: `wdc-65816-native`.

This fixes the physical decisions required by R2–R5 of the
[Exec compiler requirements](MIR65816_EXEC_READINESS_REQUIREMENTS.md).
It builds on the [retained lowering facts](MIR65816_LOWERING_CONTRACT.md).
Exec is standalone; GEM and foreign C interfaces need explicit adapters later.
The small model has no compatibility promise with this ABI.
Tasks use one shared 24-bit address space; private storage here describes
ownership and lifetime, without implying hardware memory protection.

The [machine-readable manifest](abi/action65816-native-v1.json) records the
version, register rules, sizes, offsets and worked examples. Assembly bridges
must generate their constants from it. Incompatible changes require a new ABI
identity. The advertised emitted subset and its qualification are tracked below.

Generate [Rust constants](../src/mir65816/abi/generated.rs) and
[assembly equates](abi/action65816-native-v1.inc) with
`python3 tools/generate_abi65816.py`. Add `--check` to verify freshness without
writing files. The [implementation slices](MIR65816_IMPLEMENTATION_PLAN.md)
track which consumers and executable checks have been completed.

## 1. Scope and CPU foundation

The first executable implementation supports ordinary procedures and fixed
signatures with integer, data-pointer and typed callable-pointer parameters and
results. Internal, public and qualified runtime calls use this same physical
convention. Records and arrays cross public interfaces through typed pointers;
by-value aggregates, REAL, variadic calls, unwinding and mixed near/far calls
require diagnostics until separately specified and implemented. These emission
restrictions do not remove existing source, NIR or MIR support.

Hardware foundation: native stacks and direct-page operands address bank zero;
native interrupts save PBR and enter bank zero; X-width changes can clear the
upper index bytes; M-width changes retain the accumulator's upper byte. Native
interrupt entry clears decimal mode. JSL saves three return bytes; RTL restores
the bank and increments the restored 16-bit PC. The interrupt return uses an
exact PC. See the [WDC W65C816S datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf),
sections 2.4–2.11, 3, 5 and 7.10–7.23. The policies below are project decisions.

## 2. Data representation

All multibyte values use little-endian byte order. Signed integers use their
existing two's-complement representation. The ABI adds no argument promotions.

| Value | Bytes | Natural alignment | Result home |
| --- | ---: | ---: | --- |
| BYTE, CHAR, byte enum, boolean | 1 | 1 | A bits 0–7; A bits 8–15 are zero |
| INT, CARD | 2 | 2 | A bits 0–15 |
| ADDRESS, SIZE | 3 | 2 | A bits 0–15; X bits 0–7 hold bits 16–23; X bits 8–15 are zero |
| Typed data pointer, callable pointer | 3 | 1 | Same 24-bit A/X split |
| LONGINT, LONGCARD | 4 | 2 | A bits 0–15; X holds bits 16–31 |
| Procedure | 0 | — | No result |

X is unspecified for one- and two-byte results; Y is always unspecified.
Boolean results are zero or one. A signed 32-bit result retains its complete
bit pattern in A/X; a 16-bit result carries no extra sign extension in X.

Records retain semantic natural layout: align each field to its declared
alignment, and round the final size to the maximum field alignment. Array
stride rounds element size to element alignment. Thus a three-byte ADDRESS
element can have a four-byte stride, while a three-byte pointer has alignment
one. The emitter consumes the resolved layout; it does not reconstruct it.
An explicitly unaligned alias is accessed at its declared address without
quietly rounding it up.

Existing descriptor objects retain the NIR-selected shape: a pointer at offset
zero, and an optional two-byte size word at offset three, for an extent of three
or five bytes and alignment one. Preserve the supplied size-word value; it is
not a new 24-bit array-length field. Descriptor cells and element storage have
separate identities. Public array interfaces pass an element pointer and an
explicit SIZE when a length is needed, rather than depending on this descriptor.

Code and data addresses retain all 24 bits. A callable points to its public
entry, without a hidden environment pointer. Null is zero. Integer conversions
retain the language rules; link placement, relocation and object-extent
overflow must be diagnosed. A continuation may not rely on PC wrapping into
another bank. Data operations spanning a bank must explicitly retain the carry.

## 3. Registers and call boundaries

Every ordinary entry, call site and normal return has:

- `E=0`, `P.M=0`, `P.X=0`, `P.D=0`: native, 16-bit A/X/Y, binary arithmetic.
- `DBR=0`; arbitrary data banks are accessed explicitly.
- `D` pointing to the current execution domain's 256-byte direct-page block.
- A/X/Y caller-clobbered. Live values across a call require invocation storage.
- C/Z/N/V caller-clobbered. The caller's I bit is preserved.
- `S` restored to its entry value immediately before RTL; RTL consumes only
  the three-byte return address. The caller releases arguments and padding.

Generated ordinary code never changes E or I. The two IRQ-state primitives in
section 9 explicitly change I. Routines may change M/X internally, but must
restore the boundary widths and account for lost index bits. Every control-flow
join has a known width state; restoring M alone does not clear A's high byte.

D remains fixed in generated code and qualified helpers. DBR may be changed
inside a helper, but must be zero before any ordinary call and on return.
Interrupts restore the interrupted DBR even if it was temporarily nonzero.
All helpers use this ABI or have a separately declared, qualified adapter.
There are no callee-saved general-purpose registers and no hidden register
argument, frame-pointer register, software stack or global argument cells.

## 4. Calls and stack alignment

### Argument layout

Let `align(n,a)` round n upward to a multiple of a. In declaration order,
argument i has size `w_i` and the alignment `a_i` from section 2:

```text
cursor = 0
offset_i = align(cursor, a_i)
cursor = offset_i + w_i
L = cursor                         # meaningful argument area including gaps
O = L if L is odd else L + 1       # reserved outgoing extent, always odd
```

Even a zero-argument call reserves one padding byte. The caller initializes
padding to zero; the callee must not interpret it. Evaluate arguments in the
language-defined order and retain values across nested argument evaluation in
the caller's invocation. Materializing the argument area does not change that
order. No compiler temporary may remain live solely in shared call scratch.

The caller's normal body S is even. It reserves O bytes below its current
frame, making `S_call` odd, then writes argument i at
`00:(S_call + 1 + offset_i)`. Direct calls execute JSL. At callee entry,
`S_entry = S_call - 3`, which is even:

| Displacement from S_entry | Contents |
| --- | --- |
| +1 | Return PC low, encoded as continuation PC minus one |
| +2 | Return PC high |
| +3 | Return PBR |
| +4 + offset_i | Argument i, lowest byte first |
| After the argument payload | Any terminal padding |

The caller owns this area; the callee treats it as read-only. Mutated or
address-taken value parameters receive private frame homes. An incoming home
is not an addressable source object and cannot escape as the parameter's
address. Normal completion restores `S_call`; caller cleanup adds O, returning
to the original even body S. The call including cleanup has net stack delta
zero. There is no red zone below S: interrupts can write there immediately.

### Frame layout and cleanup

After entry the callee reserves an **even** extent F, keeping body S even.
Frame objects have positive displacements from that body S; align displacement
one upward for the first object, then align subsequent objects normally.
Padding and spills are included in F. A frame has no mandatory saved-register
header. At body S, incoming argument i is at `F + 4 + offset_i`.

Outgoing areas are allocated below the fixed frame for each call. They cannot
be placed above local objects and then exposed by raising S: the next callee
would overwrite those objects. While S is temporarily lower, an object's
stack displacement increases by exactly that movement. Addresses already
formed from `S_body + object_offset` remain stable for the invocation.

All exits share the appropriate release sequence. A/X results must survive
both callee frame release and caller argument cleanup. One valid 16-bit
sequence for adding a known K to S while retaining A/X is:

```asm
TAY
TSC
CLC
ADC #K
TCS
TYA
```

This uses caller-clobbered Y; it does not promise flags. K is F in an epilogue
or O after a returned call. The result is already in A/X before this sequence.

The initial stack-relative implementation permits F up to **254 bytes**, since
F is even. For every emitted `d,S` access, including every accessed byte,
`1 <= d` and `d + access_width - 1 <= 255` must hold. Include incoming argument
displacements, spills, outgoing-area reservation and temporary pushes in this
check. F alone is not a sufficient proof. Reject a routine that needs another
addressing strategy; do not truncate a displacement. Assembly routines may
use other strategies while preserving the public entry/exit contract.

### Worked direct call

For `(BYTE, CARD, data pointer, LONGINT)`, offsets are `0,2,4,8`, L=12 and
O=13. Values `$7F,$1234,$56ABCD,-2` produce:

```text
7F 00 34 12 CD AB 56 00 FE FF FF FF 00
```

With caller body S=`$4000`, S_call=`$3FF3`, and S_entry=`$3FF0`.
A 16-byte callee frame gives body S=`$3FE0`; incoming displacements are
`20,22,24,28`. Frame release, RTL and caller cleanup restore `$4000`.

### Indirect calls

An indirect call must establish exactly the same return and argument layout.
There is no indirect JSL instruction. The baseline implementation synthesizes
the return and transfer addresses on the current stack; no shared jump cell
or self-modifying code is used.

After reserving/filling O, place the callable's low word in X and bank in Y
(Y high byte zero). With M/X both zero, the conceptual instruction sequence is:

```asm
PHK
PER resume-1       ; assembler resolves a same-bank PC-relative word
TYA
SEP #$20
PHA                ; target bank, one byte
REP #$20
TXA
DEC A
PHA                ; target low word minus one, two bytes
RTL                ; consumes transfer address; enters target in M=X=0
resume:
; A/X hold the result; release O while preserving them
```

The decrement is modulo 65536 with **no borrow from the bank**; target offset
zero is valid. PHK/PER establish the real caller return address. The target
sees only that three-byte return address. Peak transient use is six bytes,
three more than direct JSL, even though target entry and eventual cleanup are
identical. Link-check the PER range and same-bank continuation. A callable
value can change only after it has been captured according to source effects.

## 5. Direct-page ownership and reentrancy

Each task and the non-nested IRQ dispatcher own distinct, page-aligned
256-byte blocks in writable bank-zero RAM. Bootstrap uses its own block until
the first task is restored. D names the block; changing D does not copy it.

| D-relative offset | Bytes | Owner and meaning |
| --- | ---: | --- |
| $00–$3F | 64 | Compiler/runtime call-clobbered scratch |
| $00, $03, $06 | 3 each | Pointer scratch aliases within that same 64-byte region |
| $40 | 3 | Opaque execution-domain owner pointer, initialized by bootstrap/Exec |
| $43 | 1 | Domain kind: task=0, IRQ=1, bootstrap=2 |
| $44 | 2 | Lowest permitted ordinary S (`stack_floor`) |
| $46 | 2 | Highest allocated stack byte (`stack_ceiling`) |
| $48–$FF | 184 | Reserved, initially zero; unavailable to v1 compiler/helpers |

Compiler code and helpers may clobber only the scratch region. They do not
modify the owner, kind or bounds. No scratch value survives an ordinary call
unless first copied to invocation storage. Recursion and helper-to-helper
calls obey this rule, including partial pointers and arithmetic state.

Scratch **may** be live across asynchronous suspension: its entire domain block
stays allocated and untouched while another task or the IRQ dispatcher runs.
IRQ callbacks use the IRQ block. Task callbacks use their task's block and
obey ordinary call clobbers. An interrupt must never call Action! while still
using the interrupted domain's block. NMI uses no direct-page workspace.

This is the complete hidden-state inventory: CPU registers, active hardware
stack contents and this domain block. Immutable helper tables can be shared.
Any additional mutable runtime state requires an ABI inventory update and
reentrancy qualification; it cannot be introduced as an undocumented global.

## 6. Interrupt and suspended-context layout

Native IRQ/COP/NMI entry leaves a four-byte CPU frame. Before using any register
as workspace, the assembly stub performs:

```asm
REP #$30            ; hardware already saved the original P
PHA                 ; full A, including B if interrupted with M=1
PHX
PHY
PHD
PHB
```

Let `saved_s` be S after these nine additional bytes. The complete 13-byte
frame has the following displacements; all words are little endian:

| From saved_s | Bytes | State |
| --- | ---: | --- |
| +1 | 1 | DBR |
| +2 | 2 | D |
| +4 | 2 | Y |
| +6 | 2 | X |
| +8 | 2 | Full A |
| +10 | 1 | Original P |
| +11 | 2 | Exact resume PC |
| +13 | 1 | PBR |

The original S is `saved_s + 13`. E is not encoded: all these frames require
E=0. The minimal assembly-visible suspended-context record is an aligned
two-byte `saved_s` field at offset zero. Exec can embed it in a larger task
record. It refers to stack memory and the D block saved in that frame; copying
only the field or register bytes does not copy an independent task context.

### IRQ and cooperative yield

Ordinary IRQ dispatch is non-nested. After saving the frame, the entry stub
captures saved_s in a register and switches S to an even IRQ-stack body base.
It then sets D to the IRQ domain block and DBR to zero; any normalization pushes
use the IRQ stack and count towards its bound. It establishes M=X=0, decimal
clear and I=1 before calling Action!. No unsaved register or interrupted scratch
is used.

The assembly/Exec hook has the ordinary v1 signature:

```text
CARD __a816_dispatch_v1(CARD interrupted_s, BYTE reason)
```

`reason=0` means IRQ; `reason=1` means cooperative yield. The return value in A
is the selected saved_s. Returning the input resumes the same context. Exec
owns interrupt acknowledgement, current-task bookkeeping and scheduling policy.
The wrapper completes its IRQ-stack call cleanup and publishes the selected
task before loading its S. No IRQ invocation remains suspended on the shared
IRQ stack. Calls in this domain must retain I=1 and cannot block or yield.

`__a816_yield_v1()` is an ordinary zero-argument procedure callable only in the
task domain with I=0. Its assembly body executes `COP #$00`; the COP vector uses
the same save/dispatch/restore protocol with reason 1. On resumption it returns
with RTL, and its caller performs normal argument cleanup. A call with I=1 or
from IRQ/bootstrap must fault without scheduling. Other COP signatures, BRK
and ABORT are outside the resumable v1 switch protocol and reach terminal
platform fault handling. No scheduler implementation is implied here.

With I=1 and E=0, load the selected saved_s into S, ensure M=X=0, then restore:

```asm
PLB
PLD
PLY
PLX
PLA
RTI
```

Do not restore the old width flags before the full register pulls. RTI restores
the saved flags and exact execution point. PBR/PC are CPU-frame state even
though the earlier MIR switch-state enum did not list PC explicitly.

### NMI and transition windows

NMI is a bounded assembly-only handler. It saves/restores the same full frame
on whichever stack was interrupted and uses registers and bounded stack-local
scratch only. Device acknowledgement uses explicit addresses independent of
the interrupted DBR. It performs no Action!/runtime calls, neither switches
tasks nor reads current-task/IRQ transition bookkeeping. Thus it remains valid
between changes to S and D, during partial saves/restores and while I=1.

There is at most one active NMI. The board must guarantee this through source
gating or a proven minimum interarrival interval covering the entire handler;
SEI does not provide that guarantee. A board lacking it needs an extended NMI
contract before enabling that source. Interrupt code must preserve all
interrupted scratch, modes and state, including when the suspended instruction
belongs to a prologue, epilogue or helper.

## 7. Constructing the first task

The bootstrap enters native mode, supplies the stack/domain allocations,
initializes static data and zero-fill once, and installs vectors before enabling
their sources. No Atari OS, GEM or host runtime entry convention is inherited.

The task entry has the ordinary v1 signature `PROC entry(data-pointer argument)`.
Choose an even empty-body S0 within the task stack, allocate its D block, and
place the following image in ascending memory order:

```text
saved_s +  1 .. +13 : saved CPU frame, PC/PBR = exact task entry
saved_s + 14 .. +16 : synthetic RTL return to __a816_task_return_v1
saved_s + 17 .. +19 : three-byte argument
saved_s = S0 - 19
```

Here L=O=3, S_entry=S0-6, and RTI consumes the 13-byte saved frame to establish
that entry S. Initialize A/X/Y=0, D=task block, DBR=0, and P=$00 for IRQ-enabled
entry or P=$04 for a deliberately masked bootstrap task. Both M/X and decimal
are clear. The synthetic RTL return stores the stub's bank and
`(stub_PC - 1) & $FFFF`, without borrowing from the bank.

For S0=`$6000`, D=`$2200`, entry=`$12:8000`, return stub=`$00:9000` and argument
`$345678`, saved_s=`$5FED`. The 19 bytes at `$5FEE` are:

```text
00 00 22 00 00 00 00 00 00 00 00 80 12 FF 8F 00 78 56 34
```

An ordinary return from the task reaches `__a816_task_return_v1` with S=S0-3.
This assembly continuation releases the three argument bytes, then makes a
normal zero-argument v1 call to the nonreturning `__a816_task_exit_v1()` binding.
It must not RTL again. A returning exit binding reaches a terminal trap.
The creator validates space for this image, the entry's stack cost and the
interrupt reserve before making the context runnable.

## 8. Bank-zero allocation and stack limits

The board link profile supplies nonoverlapping reservations for vectors and
entry stubs, bootstrap stack/DP, IRQ stack/DP, each task stack/DP, and terminal
fault workspace. Exclude ROM, MMIO, DMA-owned regions and board-reserved RAM.
Native vectors occupy their architectural bank-zero addresses; their handlers
must be reachable there. Actual free RAM addresses are board inputs, not ABI
constants. A full 256-byte DP block must fit in bank zero at a 256-byte boundary.
Stacks also stay within bank zero and never rely on 16-bit wrapping.

Let a stack's allocated interval be inclusive `[lo,hi]`. For a task/bootstrap
stack reserve `H = 26 + NMI_extra` bytes below ordinary use: a complete IRQ/COP
save (13), a nested NMI save (13), and the NMI's declared additional stack use.
The IRQ stack needs `H = 13 + NMI_extra`. Set
`stack_floor = lo - 1 + H`, and `stack_ceiling = hi`; reject arithmetic wrap.
The task S0 and IRQ body base are even and at or below hi. Unused boundary
bytes can serve as guards, outside the published available interval.

Checked Exec code must verify that each impending reservation's lowest S is
at least stack_floor **before** changing S or pushing. A proven combined check
can cover several operations. Account for F, O, direct-call return bytes (3),
indirect transfer peak (6), temporary pushes and every callee's own checks.
The entry stubs above have no extra pushes before switching stacks; changing
that sequence changes H. The NMI handler has a published instruction/stack
bound. Overflow transfers with JML to `__a816_stack_overflow_v1`, without
changing S or first making an unchecked call. This raw, nonreturning assembly
entry receives A=requested additional bytes and X=unchanged S; Y is unspecified.
E/M/X, decimal, D and DBR satisfy the ordinary boundary, and I is unchanged.
The handler disables IRQ and uses reserved headroom or a separately reserved
fault stack/domain. It never attempts an ordinary return to the failed
reservation. The stack-check computation must detect subtraction underflow as
well as a result below stack_floor.

The map records final F, incoming offsets, spills, per-call O, transient peaks,
domain reserve and stack-check coverage. Recursive/unknown indirect call depth
is reported as unknown unless an explicit depth bound is supplied. The 254-byte
frame strategy does not limit the native hardware stack to 254 bytes.

## 9. Effects, critical sections and helpers

Ordinary calls preserve I. Two explicitly identified runtime bindings differ:

| Binding | Signature | I-bit effect |
| --- | --- | --- |
| `__a816_irq_save_disable_v1` | BYTE result, no arguments | Return token 0 or 4 containing the prior P.I bit, then leave I=1 |
| `__a816_irq_restore_v1` | Procedure, one BYTE token | Restore only I from that token |

Both are full compiler memory barriers. The save operation must capture the
old I state before SEI; the restore operation must not PLP an arbitrary token
and thereby alter M/X/decimal. All other boundary rules, argument layout and
cleanup still apply. Nested save/restore uses tokens in reverse order and
retains an outer disabled state. They provide no exclusion from NMI or DMA.

Qualified ordinary helpers can run on task and IRQ domains with the same
scratch/call rules; none can run from NMI. Shared application state still needs
explicit synchronization. Volatile accesses retain the declared width, byte
order and effects; the compiler cannot widen a byte MMIO operation. An ordinary
multi-instruction update is not atomic. No wider bus atomicity is promised.

The first helper set uses explicit loads/stores for copies. MVN/MVP emission
remains disabled until its restart, count/width and DBR effects are qualified
under asynchronous suspension. Adding a helper never exempts its mutable
workspace from section 5.

## 10. Assembly and object interface

Every separately linked object declares ABI identity/version, target, byte
order, pointer widths and its imported/exported signatures. A binding supplies
an explicit, case-sensitive ASCII linker symbol using
`[A-Za-z_][A-Za-z0-9_]*`; no implicit underscore decoration or local numeric
routine ID becomes a public name. Front-end binding resolution associates the
name with a stable routine/symbol identity before MIR. The physical emitter
does not rediscover source name resolution.

Export/import manifests specify parameter types, widths and alignments,
argument offsets, result home, calling convention, return behavior, effects,
and allowed execution domains. Assembly routines must also publish stack peak
and bounds-check coverage. Unknown effects are conservative memory barriers.
The linker rejects incompatible signatures/ABI identities and unresolved
bindings. Function-pointer relocations name the public entry. An independently
assembled caller/callee follows sections 3–4 exactly, including the padding
byte for zero-argument calls. Source binding syntax and object encoding are
implementation work; this contract does not claim they already exist.

Vector stubs, first-context restore, terminal faults and the task-return
continuation have the special contracts stated here; they are not ordinary
function-pointer targets. Generated assembly constants must include the ABI
version, domain offsets and saved-frame offsets from the JSON manifest.

## 11. Required implementation changes and proof

The [call planner](../src/mir65816/lower.rs) now retains aligned argument homes,
exact native result lanes, transfer peaks and the boundary/state inventory.
Original aggregate interfaces remain explicitly outside v1 qualification even
after their abstract expansion into physical pointer arguments.

Native fixed frames now have even extents up to 254 bytes. Outgoing argument
areas sit below the fixed frame for each call. Incoming parameter homes retain
checked body-relative displacements, including the three-byte return address.
The MIR verifier checks object extents/alignment, argument layouts, return
cleanup, caller/callee agreement and the required saved-state inventory.

The [checked stack operations](../src/mir65816/abi/stack.rs) include the last
accessed byte and temporary movement of S. Plans publish a minimum stack peak
from the fixed frame and known calls, with `allocation_complete` false. The
[scalar emitter](MIR65816_EMISSION_CONTRACT.md) now produces a separate allocated
frame, checks concrete accesses and emits bounds checks before frame/call
reservations. Its reported local peak includes all temporary slots and direct
transfers; platform interrupt headroom and each callee's reservations remain
separate obligations.

Generated Rust and assembly constants share the versioned JSON manifest. The
separate small-model policy is retained. See the
[implementation plan](MIR65816_IMPLEMENTATION_PLAN.md) for completed slices
and their checks.

Direct scalar calls, assembly interoperability in both directions, zero
arguments and the worked layout have executable coverage. Continue with:

1. Execute the indirect transfer at target offsets `$0000` and `$FFFF`, across
   banks, with a balanced stack and intact A/X result after cleanup.
2. Assemble the save/restore wrappers and fabricated task image; verify exact
   register, PC/status, stack and domain-memory restoration. Exercise ordinary
   task return, COP yield and non-switching IRQ dispatch.
3. Run two tasks through the same recursive routine/helper. Inject IRQ/NMI in
   every supported width, call, save/restore and stack-transition sequence;
   test live DP scratch and address-taken locals. Respect the nesting policy.
4. Test nested IRQ tokens, volatile traces, overflow before writes, rejected
   frames/links/bindings and the complete G1–G6 corpus from the requirements.

Layout/verifier tests and the scalar execution corpus have different scopes. The
[CPU checkpoint's timing limits](MIR65816_CPU_EXECUTION_CHECKPOINT.md) remain
qualification work before claiming asynchronous Exec readiness.
