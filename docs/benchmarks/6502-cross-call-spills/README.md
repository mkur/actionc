# Call-preserved byte spills: TN qualification

2026-11-16. Extends the
[placement contract](../../MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#loop-carried-compiler-spills)
to private byte spills spanning blocks and ordinary leaf calls.

TN shrinks from **9,824 to 9,815 bytes** against `3e38eb01`, using optimized
MIR6502, ActionCart and origin `$3000`. Only Sort changes size:

| Storage or code | Before | After |
| --- | ---: | ---: |
| Sort instructions | 246 bytes | 238 bytes |
| Sort inline storage | 1 byte | 0 bytes |
| `gap` (compiler `spill96`) | `$33B3` | `$EC` |
| `h` | `$E2/$E3` | `$E2/$E3` |

Eight accesses use ZP encoding and the inline spill byte disappears. See
[Sort's resulting assembly](sort.asm) and the [measurement receipt](qualification.json).
Other routines retain their code and storage sizes. The final compiler's CLI
image is byte-identical to the diagnostic assembly image.

The existing combined cross-call placement trial discovers spills after core
coloring. It allocates before late cleanup and its preservation replay, then
rechecks callee writes, scratch conflicts and physical observations. Strict
whole-image acceptance and exact rollback remain unchanged. There is no extra
core materialization or per-spill trial; one additional core layout measurement
supplies authoritative original spill addresses.

The independent `cross_call_spills.act` control uses a hot loop value before and
after a leaf call and across a conditional join. It exercises the same spill
path through ordinary NIR optimization, without altering promotion policy.

Validation passed:

- 52 placement unit tests, including four new spill controls. The final replay
  control passed after correcting its injected-clobber test setup.
- Two existing loop-carried spill controls and four core-continuation checks.
- 16 placement integration tests; one optional MADS test remains ignored.
- Seven cross-call VM tests and the TN Sort VM control in both runtimes.

The new control's actual parsing/loading paths cover LF and CRLF, ActionCart and
Standalone, and two origins. Full compiler and VM suites were not run.
