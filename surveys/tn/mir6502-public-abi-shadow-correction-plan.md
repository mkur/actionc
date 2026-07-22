# MIR6502 caller-shadow ABI correction plan

Status: implemented.

Date: 2026-07-22.

Scope: remove MIR6502's invented caller-side `$A0-$A2` argument mirrors and
restore the original Action calling convention. This is an ABI correctness
fix, not a speculative optimization.

## Implementation result

The implementation history is:

1. `7e3fa9c` documents the original ABI and this correction plan.
2. `62bf316` centralizes canonical Action argument homes, removes mirror
   generation, and makes the MIR verifier reject noncanonical call bindings.
3. `28ef464` deletes the 469-line compensating machine-entry demand pass and
   all lowering state used only by that pass.
4. `43992fc` adds fixed-address and indirect boundary checks, an explicit raw
   callee save-across-call fixture, and a corrected classic/MIR6502 VM probe.
5. The final slice measures TN and updates the ranked backlog in this note and
   `mir6502-listing-reanalysis-2026-07-22.md`.

On TN, relative to the 11,955-byte scaled-Y baseline:

| Metric | Before | After | Change |
| --- | ---: | ---: | ---: |
| Load file | 11,955 bytes | 11,706 bytes | -249 bytes |
| `$A0-$A2` homes on call lines | 66 mentions in 35 calls | 0 | -66 |
| Recognized instructions | 4,831 | 4,710 | -121 |
| Recognized instruction bytes | 10,956 | 10,703 | -253 |
| `LDA` + `STA` instructions | 2,457 | 2,340 | -117 |
| Branch-over-`JMP` veneers | 35 | 33 | -2 |

The final TN load-file SHA-256 is
`77f2c1a7374fbb5e936e8019784e2e86bb6789bb71dd31d94deb0d3b81ae5526`.
The modern/classic load file remains 10,445 bytes with SHA-256
`3caefd677ab3d1489e39fcc0200126b442a15278b26a9cb5351434a1c8674f39`.

All TN `machine` records are byte-identical to the pre-correction MIR; their
common SHA-256 is
`41eb669bed0789a8ee0ca6e6e0107c89c5372c4fdd83f63301cadf4f399dca81`.
The 62 `$A0-$A2` mentions that remain inside those records are authored
machine-code scratch accesses and are intentionally preserved.

The byte-exact classic `ABICALLS.COM` and `STRNAM.COM` probes retain SHA-256
`8b62d0fde3cf6638ebd1721f36cf0ae2c8657a4f93c3541517a9403cd5ddc272`
and
`4ca2deb4867d01eff34c176cb8b9c5a76c25bac8a0b92f27d18660e552883efc`,
respectively. The corrected KALSCOPE boundary fixture passes in the VM with
both classic and MIR6502: its machine callee saves A/X to `$A0/$A1`, makes a
nested call that clobbers the registers, and reloads the saved values.

## Corrected contract

Action argument values are flattened into bytes from left to right. Words are
low-byte first. Their homes are:

| Byte offset | Incoming home |
| ---: | --- |
| 0 | A |
| 1 | X |
| 2 | Y |
| 3 | `$A3` |
| 4 | `$A4` |
| n, for n >= 3 | `$A0+n` |

The caller does not copy A/X/Y into `$A0/$A1/$A2`. A current-location `=*`
routine uses exactly the same argument homes; `=*` controls entry placement and
patchability, not argument placement.

There are three distinct operations that must not be conflated:

1. A caller places bytes 0-2 in A/X/Y and later bytes in `$A3+`.
2. A normal high-level callee captures those values into its private parameter
   frame, directly for one or two bytes or through `SArgs` for larger frames.
3. A handwritten machine callee may explicitly store A/X/Y into any scratch
   address, including `$A0-$A2`, when its algorithm requires it.

Only operation 1 is the call ABI. Operation 3 is ordinary authored machine
code and must remain untouched.

## Evidence

The original-compiler probes provide direct evidence:

- `ABICALLS.COM` calls `ThreeBytes(1,2,3)` with `LDY #3`, `LDX #2`, `LDA #1`,
  and `JSR`, with no `$A0-$A2` stores.
- Its mixed-argument call places byte offsets 3-5 in `$A3-$A5` before loading
  Y, X, and A for offsets 2, 1, and 0.
- `STRNAM.COM` is byte-identical between the VM capture and compatible/classic
  generation. Calls to public `Open=*` and `Xio=*` do not initialize
  `$A0-$A2`. Their bodies themselves execute `STX $A1` as a scratch save.
- TN's `Block` body begins `PHA; STX $A0`; the `$A0` value it later consumes is
  the callee's saved X, not a caller-provided shadow.

The existing MIR6502 `Capture` tests prove only the behavior that actionc
implemented; they are not original-compiler evidence and encode the wrong
contract.

## Current defect

`src/mir6502/call_plan.rs` currently detects current-location user routines and
prepends duplicate `MirCallArg` bindings for `$A0-$A2`. The real A/X/Y bindings
are then appended. `src/mir6502/public_abi.rs` attempts to remove duplicates
whose values a shallow machine-entry scan cannot observe.

That demand pass mitigates the extra code but cannot make the model correct.
Unknown bytes, calls, branches, or direct `$A0-$A2` reads retain mirrors that
the original caller would never emit. It also turns authored machine scratch
accesses into false ABI requirements.

At the current TN head:

- 35 call operations still contain an `$A0-$A2` argument home;
- those calls contain 66 such home mentions;
- the current MIR6502 load file is 11,955 bytes after scaled-Y addressing.

All 66 call-home mentions are invalid. Explicit `$A0-$A2` operations inside TN
machine blocks are valid and outside the removal scope.

## Architectural boundary

This correction belongs in MIR6502:

- NIR carries typed call arguments, signatures, and current-location entry
  facts but does not select 6502 argument homes.
- MIR6502 owns the A/X/Y and fixed-zero-page calling convention.
- Emission consumes already selected homes and must not infer ABI placement.

No NIR or SemIR form should change. The current-location fact remains necessary
for routine layout and address identity, but it must not be passed into call
home selection.

## Implementation slices

Each slice should be independently committed with a green relevant test set.

### Slice 1: Canonicalize call homes and lock the invariant

Implemented by `62bf316`.

Goal: make one canonical Action ABI home function authoritative for lowering
and verification.

Changes:

1. Move or expose `action_abi_arg_home` and its byte-home mapping from
   `call_plan.rs` through the MIR6502 ABI module.
2. Make `plan_call` produce only the canonical argument bindings. Remove
   `mirror_public_homes` and `public_action_abi_shadow_home`.
3. Stop passing the set of current-location routine names into `plan_call`.
4. Extend MIR verification for `MirOp::Call`: walk arguments in byte order and
   require each home to equal the canonical home for its offset and width.
   Optional trailing arguments remain legal; inserted duplicate leading homes
   do not.
5. Replace the three tests in `src/mir6502/mod.rs` that require or conditionally
   elide public shadows. Add focused coverage that:
   - an ordinary and a `=*` routine with the same signature receive identical
     call homes;
   - byte offsets 0-2 use only A/X/Y;
   - a fourth and later byte still uses `$A3+`;
   - a raw machine body may explicitly execute `STA/STX/STY $A0-$A2`, but its
     caller does not.
6. Add a negative verifier test for a direct Action call containing an extra
   `$A0` binding.

Commit boundary: call planning, verifier invariant, and focused tests are green.
The old demand pass may remain temporarily; with canonical calls it should be a
no-op.

### Slice 2: Remove the compensating demand subsystem

Implemented by `28ef464`.

Goal: delete code that exists only to support the false shadow model.

Changes:

1. Remove `src/mir6502/public_abi.rs` and its module declaration.
2. Remove `public_action_abi_routines` and
   `public_action_abi_routine_ids` construction from MIR lowering.
3. Remove the post-lowering `elide_unobserved_shadow_args` invocation.
4. Remove `routine_has_current_location_address` from MIR lowering if it has no
   remaining caller; do not remove the underlying NIR entry/address fact.
5. Remove imports, parameters, machine-byte decoding helpers, and statistics
   used only by the obsolete pass.

Commit boundary: no `shadow`, `mirror_public`, or `public_abi` call-planning
implementation remains, and MIR6502 unit tests are green.

### Slice 3: Refresh fixtures and compatibility coverage

Implemented by `43992fc`.

Goal: ensure the correction survives integration and does not alter the parts
of the ABI that were already correct.

Changes and checks:

1. Refresh only MIR6502 snapshots whose call bindings or emitted bytes change.
   Classify every change as this ABI bug fix.
2. Keep `raw_machine_param_entry` in its canonical form:
   `CARD dst -> A:X`, `CARD src -> Y:$A3`, `BYTE len -> $A4`.
3. Add or adapt a runtime probe whose raw callee explicitly saves register
   arguments before nested calls, following `Open`/`Block` style.
4. Assert that calls with three or more parameter bytes still invoke the
   callee-side `SArgs` prologue and that its frame metadata is unchanged.
5. Assert that external fixed-address and indirect Action calls retain the same
   canonical argument mapping.
6. Confirm that classic-backend probe output remains byte-identical; this
   change is MIR6502-only.

Commit boundary: fixture, call-ABI integration, and runtime compatibility tests
are green.

### Slice 4: Measure TN and correct the optimization backlog

Implemented by the measurement and documentation recorded above and in
`mir6502-listing-reanalysis-2026-07-22.md`.

Goal: record the realized correction and separate it from routine-effect work.

Checks:

1. Generate pre-materialized MIR, materialized MIR, listing, map, and load file
   for TN.
2. Require zero `fixed_zp $A0`, `$A1`, or `$A2` homes on `MirOp::Call` lines.
3. Verify that explicit `$A0-$A2` bytes in machine blocks are unchanged.
4. Compare the new load size with the 11,955-byte baseline and record the exact
   delta rather than projecting a per-lane value.
5. Re-run listing-quality analysis because removing staging can expose or hide
   adjacent load/store and branch opportunities.
6. Update the ranked TN backlog. Keep transitive known-call summaries only as
   an independent prepared-address/register-effect opportunity.

Commit boundary: measured TN note and backlog are consistent with the corrected
ABI.

## Verification matrix

Run at minimum:

```sh
cargo fmt --check
cargo test mir6502::
cargo test --test mir6502_call_abi
cargo test mir6502_fixtures_match_snapshots
cargo run --bin actionc-mir6502-sweep -- --emit-load fixtures/nir
cargo test
```

Also regenerate the exact original-compiler comparison probes used as the ABI
oracle where the local workflow supports it. The expected classic artifacts
must remain unchanged.

For TN:

```sh
cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-mir6502 \
  samples/tn/modern/TN.ACT > target/TN-pre.mir

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend mir6502 --emit-load \
  samples/tn/modern/TN.ACT > target/TN.xex

rg '^  call' target/TN-pre.mir | rg 'fixed_zp \$A[012]'
wc -c target/TN.xex
```

The first `rg` command must produce no output.

## Non-goals

- Do not remove or rewrite explicit `$A0-$A2` accesses inside machine blocks.
- Do not change byte offsets 3 and later; they still begin at `$A3`.
- Do not change `SArgs`, parameter-frame layout, return homes, or optional
  trailing-argument behavior.
- Do not change current-location entry placement, address identity, or routine
  retargeting semantics.
- Do not add 6502 ABI facts to NIR.
- Do not implement transitive known-call effects as part of this correction.

## Risks and fallback policy

Source written specifically for actionc may have started relying on the
non-original mirror extension. Such a raw routine can be made portable by
explicitly storing A/X/Y where it needs them, just as the original TN library
does. Preserving that extension by default would retain the compatibility bug,
so no conservative mirror fallback should remain.

Unknown or indirect calls do not need a shadow fallback: their argument homes
are still defined by the Action ABI. Conservatism applies to their effects after
the call, not to inventing additional inputs before it.

## Completion criteria

The correction is complete when:

- one canonical Action argument-home function is used by planning and checked
  by verification;
- current-location status has no influence on call argument homes;
- no caller-side `$A0-$A2` mirror generator or demand-elision subsystem remains;
- explicit machine-code scratch saves remain byte-for-byte intact;
- TN has zero `$A0-$A2` call homes;
- `$A3+`, `SArgs`, optional arguments, external calls, and indirect calls retain
  passing coverage;
- all MIR6502 fixtures, sweeps, runtime probes, and the full test suite pass;
- the measured TN size and listing changes are documented.
