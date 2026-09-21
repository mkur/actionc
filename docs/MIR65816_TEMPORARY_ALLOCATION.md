# Native 65816 temporary allocation and stack pressure

## Baseline and reproduction

The inspected main revision is `90bd73e`, rather than Exec816's `c2268b7` pin.
Main already allocates eligible pointer-only leaves in three DP slots. Its
general emitter still reserves a distinct stack home for every MIR temporary,
followed by dedicated parallel-edge staging slots.

The native `stack_allocation` execution target supplies input 13 after linking,
checks independently specified results in both IRQ entry states, and measures
the actual emitted bytes on the qualified VM. Code size sums all routines;
cycles include the independent caller; observed stack use is entry S minus the
lowest S over the complete call chain, including arguments and return addresses.
These are representative compiler probes, not measurements of hosted Exec DOS.

| Probe | Mode | Code bytes | VM cycles | Observed stack bytes | Worker fixed frame |
| --- | --- | ---: | ---: | ---: | ---: |
| Scalar chain | raw | 586 | 891 | 52 | 36 |
| Scalar chain | optimized | 271 | 381 | 22 | 6 |
| Loop rotation | raw | 544 | 2867 | 60 | 44 |
| Loop rotation | optimized | 560 | 2857 | 54 | 38 |
| Recursive sum | raw | 539 | 5950 | 346 | 18 |
| Recursive sum | optimized | 527 | 5596 | 290 | 14 |
| Wide indirect call | raw | 834 | 1687 | 94 | 54 |
| Wide indirect call | optimized | 756 | 1503 | 62 | 26 |

The existing unlink probe confirms main's DP path: raw 191 bytes / 324 cycles /
6 frame bytes; optimized 129 / 189 / 0. Its cycles cover routine entry through
RTL, a different measurement interval from the table.

Reproduce with:

```sh
CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 tools/native65816-runtime-tests/qualify.py \
  --test stack_allocation --test pointer_allocation -- --nocapture
```

## Focused slice: reuse private stack temporaries

MIR65816 owns allocation from typed def/use facts and CFG edges. Use fixed-point
backward liveness, including loops, edge arguments, block parameters, indirect
call targets, address bases/indexes and return values. Allocate deterministic
aligned stack homes with reuse only for noninterfering temporaries. An operation's
inputs, outputs and other live values coexist for its entire machine sequence:
bytewise casts, pointer formation and carry chains must not overwrite a dying
input early. Include dead results and dead block parameters because emission
still writes them. Keep the existing source-saving parallel-copy staging area
separate from all temporary homes.

Automatic frame objects, mutable parameters and addressable locals retain their
dedicated homes. Only non-addressable MIR value temporaries share bytes. No
pointee-load forwarding, memory reordering or new alias assumption is needed.
Every value live across a direct, indirect, recursive or helper call stays on
its invocation's stack. Recheck the allocation before selection, including
physical byte overlap, widths, frame ownership and accounting. Derive incoming
displacements, spill bytes and local peak from the final allocation; preserve
all last-byte access checks, the 254-byte even-frame strategy, call guards and
platform interrupt reserve. Public ABI v1 and image v3 remain unchanged; image
maps can already give several temporary IDs the same physical location.

Qualify actual raw and optimized machine code for loops/parallel copies,
recursion, direct/indirect calls, helpers, mixed widths, aliasing and stack
boundaries. Exercise an assembly callee destroying all scratch and registers
while the caller has a live value. Reuse the existing two-task IRQ-at-each-site
and seeded IRQ/NMI suites to cover suspension during reused-slot operations.

## CPU register and DP opportunities

The 64-byte scratch area is per execution domain, but all of it and A/X/Y are
call-clobbered. The general selector currently uses D+0 for pointers, D+8 for
results, D+16 for right operands, and D+20 for indexing; aggregate copies also
use secondary pointers. The bounded pointer-leaf selector has a separate,
verified scratch whitelist. It cannot simply be enabled around other operations.

Further allocation needs explicit per-operation and helper scratch/register
clobber contracts, with homes split or spilled before calls. Initially allocate
only values whose entire lifetime fits between barriers, preserving the full
operation's input lifetime. A/X/Y retention additionally needs width/flags and
addressing constraints; X/Y and A are already working registers within selection.
Using otherwise unused DP bytes is plausible after that audit, but does not
eliminate the need for invocation-owned homes across calls or recursion.

Preemption safety depends on the existing platform contract: each task and IRQ
domain owns its own DP, the bridge restores the full CPU state, and NMI preserves
the interrupted scratch without calling Action! or switching tasks. Neither DP
allocation nor register retention permits shared scratch between suspended
domains or removal of interrupt headroom. Stack reuse reduces pressure without
extending those contracts. Exec816 adoption still needs image-v3 packaging and
hosted call-chain qualification; this slice does not change its pin or budget.
