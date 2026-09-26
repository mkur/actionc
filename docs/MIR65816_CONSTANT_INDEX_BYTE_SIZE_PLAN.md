# MIR65816 constant indexes, BYTE consumers and argument constants

Status: all five slices implemented. Measured saving: **11,869 bytes (11.6 KiB)**.
The loaded-size estimate is **270,269 bytes (263.9 KiB)**, leaving **8,125 bytes**
to the cap. See the [completed measurements](benchmarks/65816-constant-index-byte/README.md).

Implement five independently measured commits after compiler `a25fa91d`, using
the frozen Exec `622b139-dirty` workload (631 routines, 120 input hashes).
The baseline loaded-code-plus-initialized-data estimate is 282,138 bytes,
excluding 72,252 guard bytes; the remaining gap to 256 KiB is 19,994 bytes.
This is guard subtraction, not a separately linked guard-disabled release.

## Slices

1. **Constant indexes through captured pointers.** For nonvolatile scalar
   loads/stores, fold unsigned numeric index × stride + displacement with wide
   host arithmetic. Admit only complete captured pointer/payload homes and an
   offset whose complete payload fits Y. Keep full 24-bit base arithmetic and
   exact ascending external traffic; retain generic selection on overflow,
   unsupported values or volatility. Preserve existing direct-symbol selection.
   Model: roughly 4 KiB, including zero indexes previously scaled at runtime.
2. **BYTE/word equality zero tests.** Normalize zero to the right for Eq/Ne.
   Use the exact-width load's Z instead of CMP #0. A forwarded word requires
   an existing checked N/Z witness or an explicit load; never trust ambient
   flags. Preserve ordering comparisons and materialized Boolean results.
   Model: 1.2–1.5 KiB.
3. **Adjacent BYTE load→compare forwarding.** Plan a nonvolatile BYTE load
   and its immediately adjacent sole-use Eq/Ne consumer together. Retain the
   original exact external read, consume A directly, and omit only the private
   temporary store/reload. Preflight operands/homes before removing a definition;
   retain allocated homes and conservative barriers. No cross-block, cross-call
   or alias-sensitive forwarding. Cover fused branches and Boolean results.
   Model: up to 2.4 KiB across 604 candidates.
4. **Advance Y within wide indirect accesses.** Within one scalar operation,
   use INY twice instead of a second LDY immediate two bytes above the first.
   Admit only a known intact X16/Y state and complete bounded payload. Do not
   introduce cross-operation Y residency. Preserve A and exact external traffic.
   Model: about 1.6 KiB; record the extra cycle relative to LDY immediate.
5. **PEA for constant argument words.** Extend the complete call-push planner
   with a typed immediate word push independent of M. Cost mode transitions,
   retain ABI bytes/padding and guard-before-push order, and adjust stack-source
   offsets after every push. Update encoding, physical effects, tracking and
   replay together. Keep symbolic byte relocations and unsupported calls on
   their established paths. Model: at least a 533-byte instruction cohort,
   with additional mode/planning opportunities to be measured.

All selection remains target-private MIR65816. No NIR/SemIR contracts, ABI,
frame reservations, direct-page layout, external access assumptions or source
semantics change. Fallback is atomic. Estimates are not credited until measured;
budget roughly 8–10 KiB for the series, not the entire remaining release gap.

## Validation and commits

Each slice commits implementation, focused regression tests, relevant contract
updates and compact frozen-Exec size evidence. Check raw/optimized compilation,
LF/CRLF through real fixture paths, flat and rebased o65 execution, boundaries,
neighbor canaries and IRQ/NMI reentry for changed windows. New instructions need
independent ca65/VM physical-effect checks. Run affected 65816 unit/integration
targets, and debug/release runtime targets for new behavior.

Reuse the frozen inventory probe and the existing per-routine/span measurement
checks. Verify input hashes, guards, ABI, frames, stack peaks and initialized
data; report direct and cumulative byte deltas and any larger routines. Keep
bulky outputs under ignored target directories. **Do not run full/final backend
or hosted Exec qualification**, including after the last commit.

Record results in [the series report](benchmarks/65816-constant-index-byte/README.md).
