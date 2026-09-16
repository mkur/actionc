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

## IRQ-state imports

The bridge exports ordinary v1 routines with explicit IRQ effects:

| Export | Action! interface | `irq_effect` | Checked local peak |
| --- | --- | --- | --- |
| `__a816_irq_save_disable_v1` | `BYTE FUNC SaveIRQ()` | `save_disable` | 1 byte |
| `__a816_irq_restore_v1` | `PROC RestoreIRQ(BYTE token)` | `restore` | 0 bytes |

Save/disable returns the previous I bit as BYTE 0 or 4, with A's high byte zero,
and returns with IRQ masked. Restore changes only I according to token bit 2;
it never loads arbitrary processor status. Both return at the native boundary.
Nest these operations by retaining each token in its invocation. All imported
calls are conservative memory barriers in raw/optimized compilation. Import
metadata is carried in image version 3 and checked against these signatures.

The saved-status push is checked before it occurs. An IRQ between reading the
old status and disabling it may suspend the caller normally; its eventual
return still reports the original state. The critical region begins after the
save/disable call returns. NMI remains possible and follows the bounded policy
above; these routines do not protect against NMI or hardware bus agents.

## Executable checks

`tools/native65816-runtime-tests/tests/contexts.rs` executes the assembled bridge
and serialized Action! images. It checks the published 19-byte example,
invalid layouts, first entry/argument/exit, COP yield, invalid domains/masking,
unknown COP signatures, returning exit faults, and all interrupted M/X
combinations including hidden B and nonzero DBR. NMI is injected at each IRQ
bridge instruction boundary, including partial saves/restores and S/D changes.

The separate `preemption` and `effects` tests qualify two simultaneously live
tasks, shared recursive/memory helper entry, instruction-boundary IRQ scheduling,
nested tokens and volatile traces. They use the corrected CPU status timing.
See [initial Exec acceptance](MIR65816_EXEC_ACCEPTANCE.md) for exact coverage,
reproduction commands and the required board validation.
