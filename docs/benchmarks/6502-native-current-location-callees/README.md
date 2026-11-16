# Native current-location callee admission: focused qualification

2026-11-16. Implements the
[three-slice plan](../../MIR6502_NATIVE_CURRENT_LOCATION_CALLEES_IMPLEMENTATION_PLAN.md).
Entry analysis is committed in `29ae10d2`; caller residence admission in
`11d16f0c`. The [receipt](qualification.json) records configuration, hashes,
selected homes, sizes, work counters and test scope.

Native `=*` leaves can now supply actual-entry requirements and write facts for
caller ZP residence. They retain `ActionObservable`, the Action call ABI, direct
entry and their existing storage policy. Planned observable capture, machine/OS
admission and nonleaf entry composition remain outside this correction.

## Independent saving

The [control](../../../fixtures/runtime/native_current_location_homes.act) calls
the same native `=*` leaf twice while preserving a word parameter, a byte
parameter and a word local. Inlining is disabled. Before/after use the same
source and optimized ActionCart configuration at `$3000`, with pre-fix and
corrected compiler artifacts.

| Component | Before | After |
| --- | ---: | ---: |
| Complete packaged image | 120 bytes | 101 bytes |
| `Hold` instructions | 79 bytes | 65 bytes |
| `Hold` inline homes | 5 bytes | 0 bytes |
| `Touch` instructions | 20 bytes | 20 bytes |
| `Touch` inline parameters | 3 bytes | 3 bytes |

The **19-byte saving** is 14 instruction bytes plus five inline bytes. `Hold.keep`
uses `$A0/$A1`, `n` uses `$A2`, and `work` uses `$E0/$E1`. The actual callee entry
still receives its arguments in A/X/Y; its own parameter cells stay inline.
The corrected compile uses one combined cross-call trial. The pre-fix control
has no eligible cross-call proposal.

The matching plain-declaration control saves the same 19 bytes (114 → 95).
A separate existing loop-carried-spill fixture also selects its spill through
a native `=*` function. Nonleaf and assembly `=*` controls retain the complete
fallback and report the corresponding unsupported boundary.

The initial positive fixture used global output symbols. Existing write
summaries conservatively let a Global write alias every ZP byte, so that version
never selected a home. The retained fixture uses fixed-address local aliases
for its observed outputs. This isolates entry admission; the global-write alias
policy was not changed.

## TN result

Optimized MIR6502 TN, ActionCart at `$3000`, remains **9,811 bytes**, byte for byte
identical to the pre-fix image and the
[retained qualification](../6502-zp-home-lifetimes/README.md). Every routine size
and mapped home is unchanged. The pre-fix executable was reused and checked
against that image; no historical compiler rebuild was needed. The retained
receipt predates optimized-default enablement in `passes.rs`; the comparison
uses the enabled defaults in both artifacts and unchanged TN sources.

`Copy` still has 673 instruction bytes and 58 inline bytes, including 48 array
bytes. Its pointer-read/variable-word-arithmetic admission guard is unchanged.
`SwapWin` still calls `SetWin`, so it requires the separate nonleaf entry and
preservation extension even after the declaration-only exclusion is removed.

All recorded compiler work counters match: one combined cross-call trial, six
materializations, 146 emission invocations, 876 inner emission probes and 17
late-cleanup preservation replays. This correction adds no placement search.

## Checks

- 58 parameter-home unit tests, including actual versus planned entry, malformed
  contracts, extra inputs, both word-lane clobbers, repeated sites, spill checks
  and exact rollback.
- 19 cross-call integration cases and 11 call-ABI cases. The affected new group
  was rerun after correcting its fixture and adding explicit native-nonleaf and
  assembly `=*` fallback controls; unrelated passing cases were not repeated.
- One grouped VM control covering ActionCart/Standalone, placement off/on and
  LF/CRLF through the actual source loader: 56 caller invocations and 112 observed
  callee entries. It checks host-derived results, A/X/Y arguments, output memory,
  sentinels, stack balance, selected homes and absence of an entry JMP trampoline.

One optional MADS listing test was not run. No emission logic, shared frontend
or NIR contract changed; no full compiler or unrelated backend suite was run.
Large diagnostic artifacts remain under ignored `build/native-current-location/`.

Focused reproduction commands (reuse the repository's shared target/profile):

```sh
cargo test --lib mir6502::parameter_homes
cargo test --test mir6502_cross_call_homes --test mir6502_call_abi
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes native_current_location
```
