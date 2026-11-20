# Implementing lifetime-aware MIR6502 spill placement

Let a private byte spill reuse a ZP byte when every occupant can keep its value for its full lifetime. Start with reuse across routines. That covers the two displaced TN spills and can land independently of sharing within a frame. Keep one binding per spill, the existing spill eligibility and pool, and the named homes already selected. Land each slice with its regression checks in a separate commit. [Design](MIR6502_PRIVATE_ZP_HOME_PLACEMENT.md#lifetime-aware-spill-placement)

## 1. Identify who needs each byte and when

Before changing selection, build optimized TN at `$3000` for cartridge and standalone. Save the input revision and local changes, flags, XEX sizes, routine sizes, spill placements and VM cycle counts for comparison. Use the same TN source and contract set for both builds of each runtime. [TN measurements](benchmarks/6502-tn-home-breakdown/README.md#measurement)

Build a per-routine interference query from the selected MIR. Identify each source byte by routine, parameter or local ID, and offset. Identify a spill by its routine and spill ID before placement and its distinct virtual slot afterwards. Reuse CFG liveness for reads, definitions and live values before and after each operation and terminator. Keep source-fixed storage, runtime reservations, explicit observations, exposed storage and dedicated persistent homes excluded. Compute the facts once per routine per program snapshot. [Logical source bytes](../src/mir6502/parameter_homes/logical.rs) · [Spill liveness](../src/mir6502/analysis/home_liveness.rs)

Exercise the query with a last use followed by a definition, a join, a loop backedge, a dead store that overwrites a live value, and a collision with just the high byte of a word. Require known read/write order for reuse within one operation. A physical scratch write can destroy a logical value and cannot initialize it. This slice should reproduce the baseline images byte for byte.

## 2. Enable sharing across routines

Replace the program-wide reservation of named homes with a check at every reachable call site. Propagate caller bytes needed across the call through native callees and their descendants. Include argument setup, entry reads, scratch, return preservation and result delivery. Check spills against named homes and previously selected spills in both caller and callee. Use matched contracts for opaque calls. Where recursion, callbacks, interrupts or unknown effects prevent a proof, exclude the affected byte for the overlapping lifetime. Keep deterministic selection and continue to the next spill when one has no available byte. [Spill discovery and verification](../src/mir6502/parameter_homes/spills.rs) · [Call facts](../src/mir6502/parameter_homes/calls.rs)

Use the same query in discovery and final verification. Rebuild it after expansion and remapping, including callers whose own frames stayed unchanged. Permit overlap at the proposed address only for identified, proven occupants. Retain checks for explicit numeric and symbolic observations against the old inline addresses, fallback layout and new byte. Use the existing placement trial, strict whole-image size comparison and rollback. [Materialization and replay](../src/mir6502/preparation.rs)

Replace the blanket sibling-rejection test with a positive sibling case and a nested call before definition or after last use. Add a second call to the same callee while the caller value is live and require rejection. Test a clobbering call inside and outside a spill lifetime. Inject a late scratch collision or changed callee footprint and verify that rollback restores the selected artifact. [Spill tests](../src/mir6502/parameter_homes/spills/tests.rs)

## 3. Enable sharing inside one frame

Keep a distinct logical identity for each remapped spill. Record which virtual slots came from spill placement so the frame, parameter and local verifiers can apply the byte-interference query to precisely those overlaps. Keep rejecting unproven virtual overlaps, permanent reservations and malformed allocations. Check late cleanup and alias queries for assumptions that distinct slots imply distinct physical bytes. [Frame verifier](../src/mir6502/verify.rs) · [Parameter bindings](../src/mir6502/parameter_homes.rs) · [Local bindings](../src/mir6502/parameter_homes/locals.rs)

Exercise spill/spill, spill/local and spill/parameter reuse through final emission. For each pairing, verify the result with disjoint lifetimes and reject a forced overlap with a live occupant. Include a loop-carried value and a store at the other occupant's last use.

## 4. Qualify TN on both runtimes

| Case | Acceptance check |
| --- | --- |
| `PopUp.d` | Keep the previous selection in ZP across `Range`. In the VM, verify menu-bar positions `0, 0, 1` when the selection advances and `0` when Escape leaves it unchanged. |
| `ArenaCursor` | Recover the byte spill used by the cursor bounds check. Exercise the calls through `Push` and `SavePos` before `d = c`, and through `RestorePos` and `Pull` after the final use. Check the restored arena cursor. |
| `Sort` | Keep the `gap` spill in ZP across `Fnamecmp` and loop backedges. Sort an unsorted directory that needs multiple gap iterations and verify the resulting order. |

The measured recovery target is **12 XEX bytes per runtime**, three in `ArenaCursor` and nine in `Sort`, with `PopUp.d` still in ZP. Accept any equally effective byte assignment. Record actual spill addresses, routine and XEX byte deltas, and VM cycles for matched workloads in the TN report. If a spill stays inline, identify the conflicting occupant and site. Keep compiler spills separate from named-home counts. [TN breakdown](benchmarks/6502-tn-home-breakdown/README.md#measurement)

Run the affected MIR6502 unit, integration and VM targets. Use the existing cycle workload for the before/after measurements.

```sh
cargo test --lib mir6502::parameter_homes::
cargo test --lib verifier_rejects_overlapping_zero_page_allocations
cargo test --test mir6502_cross_call_homes --test mir6502_parameter_homes
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes --test tn_dispatch
ACTIONC_TN_HOME_CYCLES=1 cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_dispatch default_home_panel_and_copy_cycles -- --ignored --nocapture
```
