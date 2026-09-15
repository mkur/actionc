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

## Still pending

The later [native scalar emitter](MIR65816_EMISSION_CONTRACT.md) now executes
compiler-generated native images on this bus, including checked frames and
assembly interoperability. That separate corpus covers the
[physical ABI v1](MIR65816_PHYSICAL_ABI_V1.md) scalar subset, including indirect calls.
Context entry/save/restore and asynchronous qualification remain in the
[implementation plan](MIR65816_IMPLEMENTATION_PLAN.md). The earlier
emulation-mode probe alone does not establish any native ABI property.

The port retains X65's documented REP/SEP/RTI status-update timing limitations;
two ignored timing tests remain failing reproducers. ABORT is explicitly
rejected. Exact interrupt timing, the complete SingleStepTests corpus and
Altirra comparisons of banked/timed behavior remain further qualification work.
These limits prevent treating this checkpoint as full Exec readiness.

No compiler lowering or emission implementation changed during this work.
The [jgenesis workspace](../tools/vm65816-runtime-tests/README.md) remains as
the earlier comparison experiment; actionc does not acquire a dependency on
either emulator through this checkpoint.
