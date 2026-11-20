# MIR6502 block-local spill placement at calls

Use the value live at each call to decide whether a private byte spill can use zero page. In Mandelbrot's `Iterate`, two spill bytes carry the calculation of `nextX`. Their last reads deliver `nextX` before `MulFloor`. After the call, fresh stores reuse both bytes for its result. A third spill is reused around the two squares. The early allocator currently sees calls between the first and last accesses to all three spill IDs and rejects them. All three values are dead at those calls and at block exit. The later allocator handles spills that are live at a block exit. [Mandelbrot kernel](../samples/graphics/mandelbrot/fractal/mandelbrot.act#L40) · [Early call check](../src/mir6502/materialize/spills.rs#L1896) · [Later spill selection](../src/mir6502/parameter_homes/spills.rs#L85)

Land the fix in three independently checked commits.

## 1. Fix the early placement decision

- Keep the existing block-local candidate discovery, ordering, private ZP pool and physical reservations. For a candidate previously rejected by an intervening call, establish that it is a compiler-owned, unexposed, single-byte spill. Apply the rule to every eligible routine. [Candidate selection](../src/mir6502/materialize/spills.rs#L1619) · [Storage ownership](../src/mir6502/analysis/storage_ownership.rs#L130)
- Reuse `MirHomeLiveness` on the current MIR, before remapping. Build it at most once per routine that needs the new check. Use `visit_sites` to inspect barrier reads and the set of live bytes after each barrier in one walk. Avoid a backward rescan per spill and call. Recompute on core replay because spill identities and selected calls can change. [Home liveness](../src/mir6502/analysis/home_liveness.rs#L224) · [Materialization order](../src/mir6502/materialize.rs#L1769)
- For each previously blocking `Call`, `RuntimeHelper` or `SArgsCapture`, admit the spill only if that operation does not consume the spill and the byte is dead immediately afterwards. Check explicit and projected inputs, including argument and indirect-target reads. Only a definite logical overwrite ends the old value. Retain the existing conservative decision if the facts are unavailable or argument setup could overwrite an input before it is consumed. Preserve the existing treatment of calls at interval endpoints. [Barrier classification](../src/mir6502/materialize/spills.rs#L227) · [Post-materialization effects](../src/mir6502/analysis/effects.rs#L1682)
- Prove the change through the allocator entry point with paired tests. A last use, a call, a fresh store and a later read should get ZP. Moving that later read ahead of the fresh store must keep the spill inline. Cover two calls where only one has a live value, a read-modify-write after the call, a call that consumes the spill during setup, and a previously eligible final-use argument. Exercise all three barrier kinds and retain the existing fixed-lane and pool-exhaustion checks. [Materializer tests](../src/mir6502/materialize/tests.rs)

## 2. Verify the emitted code on both runtimes

Add a small regression independent of Mandelbrot. Use a callee that overwrites the candidate scratch byte. Execute both the dead-before-call case and the live-through-call control, checking the observed result and the selected placement. Exercise the normal MIR6502 pipeline so spill coloring, ABI setup and late cleanup all participate. Assert the storage class and observable behavior without relying on a particular spill ID or ZP address. Reuse the existing cross-call integration and VM targets. [Integration tests](../tests/mir6502_cross_call_homes.rs) · [VM controls](../tools/vm-runtime-tests/tests/cross_call_homes.rs)

## 3. Qualify Mandelbrot and check for regressions

Compare the baseline and candidate compiler on identical sources with optimized MIR6502 at `$3000`. Record XEX bytes, emitted spill accesses in `Iterate`, the calls selected for the squares, and VM cycles from program entry through the last rendered row. Check all 30,720 pixels against the integer oracle on cartridge and standalone. Exercise direct calls that escape early as well as the 32-iteration case. [Program](../samples/graphics/mandelbrot/mbfixed.act) · [Recurrence and bitmap oracle](../tools/vm-runtime-tests/tests/oscar64_mandelbrot.rs)

| Runtime | Baseline → audit prototype XEX bytes | Baseline → audit prototype full-frame cycles |
| --- | ---: | ---: |
| Cartridge | 1,871 → 1,729 | 887,991,631 → 885,723,066 |
| Standalone | 2,336 → 2,216 | 900,232,963 → 879,247,133 |

These figures came from a prototype restricted to `Iterate` against revision `7e9c7fc3`. A compiler-wide fix can also change callee inlining, the selection of named homes and which spills occupy the pool. Compare the resulting frame and listings if the totals differ. Report early-exit timings separately, since one cartridge input took five extra cycles in the prototype.

Use TN as the existing regression case and Monte Carlo as an independent loop workload. Compare before/after images, run TN's menu and directory checks, and inspect any size growth or displaced hot spill. The implementation is ready when the paired placement controls pass, both full frames match the oracle, and the measured size and cycle changes are explained. [TN qualification](benchmarks/6502-tn-home-breakdown/README.md#lifetime-aware-spill-placement) · [Monte Carlo kernel](../samples/benchmarks/bench/montecarlo.act)
