# Native v1 assembly context interface

The reusable [ca65 bridge](../runtime/65816/native-v1.s) implements the
[physical ABI](MIR65816_PHYSICAL_ABI_V1.md)'s first restore, IRQ/COP entry,
NMI handler, yield and task-return continuation. It contains no scheduler.
Assemble it into bank zero with `docs/abi` on ca65's include path. It imports
only the generated `action65816-native-v1.inc` constants.

## Platform configuration

Supply these assembler constants before including the bridge:

| Symbol | Contract |
| --- | --- |
| `A816_IRQ_DP` | Aligned writable 256-byte IRQ domain in bank zero |
| `A816_IRQ_STACK_TOP` | Even ordinary IRQ body S |
| `A816_IRQ_STACK_FLOOR` | Initialized domain floor; at least six bytes below the top |
| `A816_DISPATCH` | v1 `CARD(CARD saved_s, BYTE reason)` callback |
| `A816_TASK_EXIT` | Nonreturning v1 zero-argument procedure |
| `A816_STACK_OVERFLOW` | Raw nonreturning v1 stack-fault adapter |
| `A816_TERMINAL` | Raw terminal fault entry; I=1, native M=X=0, decimal clear |
| `A816_NMI_ACK` | Explicit long byte address; NMI acknowledges by writing 1 |

The platform supplies nonoverlapping bank-zero reservations for vectors,
bridge, task/IRQ/bootstrap stacks and domains, and fault handling. The bridge
normalizes DBR on the IRQ stack. Its one-byte normalization push and six-byte
callback transfer fit the prevalidated IRQ allocation; the callback checks its
own frame/calls. Task-stack interrupt headroom remains 26 bytes and IRQ-stack
headroom 13 bytes; this NMI implementation adds zero extra stack bytes.

Install native IRQ, COP and NMI vectors at `$FFEE`, `$FFE4`, `$FFEA`. Route BRK
and ABORT to terminal platform handling; neither is a resumable context entry.
The VM explicitly rejects ABORT and does not qualify its hardware behavior.

## Domain and first-task construction

[`mir65816::context`](../src/mir65816/context.rs) supplies checked domain bytes
and `FirstTask::new`. It validates full-width addresses, bank-zero alignment,
stack/headroom bounds, initial entry cost and overlap of each domain with its
own stack. The platform must additionally check overlap between allocations.
Use the image's routine map for entry address and local stack peak. Callees
still make their own checks; recursion has no inferred whole-task bound.

Load `FirstTask.bytes` at `saved_s + 1`, and store `saved_s` in the task's
assembly-visible two-byte record. Enter `__a816_restore_v1` with selected
saved_s in A, E=0, M=X=0, I=1. The stub restores the full frame through RTI.
Task entry receives one three-byte data pointer. A normal return releases that
argument and calls the task-exit binding; a returning exit binding traps.

The creator initializes each distinct 256-byte domain once and leaves it
allocated while suspended. Saving D does not copy that memory. Compiler scratch
can remain live during suspension and belongs to that domain.

## Dispatch and nesting

IRQ and COP save the complete 13-byte frame before using interrupted registers.
They switch to the separate IRQ stack/domain, normalize DBR and decimal mode,
and call the dispatcher with I=1. The dispatcher acknowledges IRQ hardware,
publishes the selected task and returns its saved_s. Returning the input
resumes the interrupted context. Dispatch must not block, yield or enable IRQ.
No IRQ-stack activation may remain suspended after it returns.

`__a816_yield_v1` is an ordinary zero-argument import with one byte of checked
local stack use. It accepts only task-domain callers with I=0. COP entry also
validates the interrupted I bit, domain kind and zero COP signature. Invalid
use traps before calling the dispatcher.

NMI saves/restores the same full frame on the interrupted stack. Its body is
bounded: save five registers, set A8, write one byte to the configured long
address, normalize widths, pull five registers and RTI. It has no calls, loops,
direct-page use or task-bookkeeping access. The board must prevent a second NMI
until this handler finishes. SEI does not provide that guarantee. Boards whose
acknowledgement differs must replace and independently qualify the NMI body.

## Executable checks

`tools/native65816-runtime-tests/tests/contexts.rs` executes the assembled bridge
and serialized Action! images. It checks the published 19-byte example,
invalid layouts, first entry/argument/exit, COP yield, invalid domains/masking,
unknown COP signatures, returning exit faults, and all interrupted M/X
combinations including hidden B and nonzero DBR. NMI is injected at each IRQ
bridge instruction boundary, including partial saves/restores and S/D changes.

This slice establishes the assembly interface. Two simultaneously live tasks,
IRQ-state primitives and instruction-boundary scheduling qualification belong
to slice 7; the VM's status-timing limitations are not waived by these tests.
