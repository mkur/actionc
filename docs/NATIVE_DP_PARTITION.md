# Native DP partition development record

Native ABI `action65816.native.v2` preserves D+$00–$7F for a caller runtime.
Compiler scratch is D+$80–$BF; owner/kind/stack bounds occupy D+$C0–$C7;
D+$C8–$FF remains reserved zero. D still names a full aligned 256-byte page.
The generated Rust, ca65 and Action! definitions share one manifest.
No cross-language calling convention or C integration is claimed.

Compact o65 v3 (`A8C3`) retains an eight-byte header plus four bytes per import.
Native image versions 3/4 and rich o65 formats keep their existing ABI field.
ABI-bound entry, runtime and arithmetic symbols now use v2. Old native ABI
images and compact-v2 descriptors are rejected before execution.

Development checks (2026-09-27):

- ABI generator and independent o65 fixture freshness checks.
- 301 MIR65816 unit tests passed; one optional inventory test ignored.
- Backend integration: native ABI, arithmetic, emission, address selection,
  contract, native/o65 CLI and o65 relocation tests passed. Optional scalar
  inventory generation remains ignored without its external corpus.
- Native runtime: contexts, interop, memory, scalar DP, pointer preemption,
  guard branches, arithmetic helpers, o65, constant shifts, state tracking
  and instruction effects passed. Both raw and optimized source variants are
  exercised where supported by these targets. Guard tests cover IRQ/NMI during
  stack/domain transitions. Task, bootstrap and IRQ harnesses seed nonzero
  caller-workspace patterns and check preservation independently of metadata.
- Test-side operand decoders, inline assembly and traffic windows were migrated
  with the ABI. Failed old-offset fixtures were corrected and affected targets
  rerun; the already passing arithmetic/memory targets were not repeated.

Tests use the runner-pinned actionc-vm revision
`56ddc5c5de41f0e7294e87c440869550eaf53292` plus its committed status-timing patch.
This is backend development evidence, not hosted Exec or release qualification.
Historical qualification JSON remains unchanged; historical plans and reports
retain their original native-v1 identities and link to the archived contract.

The compact header revision is derived from native ABI_VERSION (2). Follow-up
o65/CLI checks passed (13 + 7 cases), including the exact eight-byte header and
rejection of a stale revision 1. The final documentation correction changes no
compiler, runtime, ABI-manifest or fixture bytes; passing execution is reused.

Bank-zero reservation delta: **0 fixed bytes; 0 bytes per task**. Page size,
alignment, guards, task stride and unused reserved capacity are unchanged.
