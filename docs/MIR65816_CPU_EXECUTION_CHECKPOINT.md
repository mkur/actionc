# 65816 CPU execution checkpoint

The X65 qualification/port sequence is implemented in the separate
[actionc-vm repository](https://github.com/mkur/actionc-vm). The CPU lives in
`crates/w65c816`; `src/native65816.rs` supplies the VM execution profile.
See that repository's `docs/65816_EXECUTION.md` and `crates/w65c816/NOTICE.md`
for commands, source provenance and hardware limitations.

## Completed

1. Pinned the original X65 C header at
   `84c0e3c4e26bc174470a5217be9f0a72b0e95c0d` and ran its 1,610-case instruction ROM.
2. Ported the CPU to safe Rust. Tests compare all 19 internal state words and
   pins on every ROM cycle and across 40,960 opcode/mode/signal-schedule cases.
   The C implementation is a test-only optional dependency.
3. Integrated a separate 24-bit cycle bus into the VM. Architectural probes
   cover banked access, widths, calls, interrupts, block moves, WAI, reset,
   snapshots, guards and bounded runs. Short NMI pulses and reset conditioning
   pass the cases that failed in the earlier jgenesis test drive.
4. Cross-checked a shared native-mode probe with local AltirraSDL. All 14 result
   bytes match. Also executed standalone compiler output from the classic
   optimized and MIR6502 backends in 65816 **emulation mode**.

## Native compiler and status-timing qualification

The [native emitter](MIR65816_EMISSION_CONTRACT.md) now executes compiled images
on this bus. The [initial Exec acceptance](MIR65816_EXEC_ACCEPTANCE.md) records
scalar/memory, ABI, two-context, asynchronous, effects and image-limit evidence
for the advertised kernel subset. The original emulation-mode probe alone did
not establish any native ABI property.

The REP/SEP and RTI status-timing defects are corrected in actionc-vm commit
`da81c1e`. REP/SEP retain the old status until their final update cycle, and RTI
applies its pulled status before the corresponding IRQ pipeline sample. The
former ignored reproducers and a focused RTI regression now pass. The corrected
production core also runs the independent 1,610-case instruction ROM.

Cycle-by-cycle equivalence against the unmodified pinned C source remains a
separate regression check in an explicit test-only original-timing mode. Native
compiler execution always uses the corrected production mode. Eight CPU tests
pass in debug/release; 30 VM native CPU/bus tests also pass. The compiler's
[qualification runner](../tools/native65816-runtime-tests/qualify.py) reproducibly
applies the committed correction to the published VM base in a private cache.

## Remaining hardware scope

ABORT is explicitly rejected. Full external CPU conformance, the complete
SingleStepTests corpus, and banked/timed comparisons on real target hardware
remain further work. The qualified compiler corpus does not establish arbitrary
interrupt timing or custom-board behavior. A native board bootstrap and
interrupt smoke test is the next platform milestone.

The [jgenesis workspace](../tools/vm65816-runtime-tests/README.md) remains the
earlier comparison experiment. The compiler itself does not depend on either
emulator; qualification runs in an isolated test workspace.
