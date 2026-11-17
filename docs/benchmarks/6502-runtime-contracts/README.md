# Runtime and library contract qualification

Qualified 2026-11-17. Production revision `0da788d6`, compared with the starting
revision `0a291ceb`. This completes slice 9 of the
[implementation plan](../../MIR6502_RUNTIME_HELPER_CONTRACT_IMPLEMENTATION_PLAN.md).
The [coverage inventory](../../MIR6502_RUNTIME_LIBRARY_CONTRACT_COVERAGE.md)
records supported contracts and remaining library dependencies.

## Measured outcome

All comparisons use optimized MIR6502, ActionCart and the same source, origin
and configuration as the recorded baseline. Packaged bytes exclude uninitialized
buffers and are not total RAM consumption.

| Program | Origin | Packaged bytes before → after | Routines with named ZP homes | Available routine entry certificates |
| --- | --- | ---: | ---: | ---: |
| TN | `$3000` | 9,921 → **9,913** | 9 → **10** | 30 → 30 |
| Huffman portable | `$2000` | 3,851 → 3,851 | 0 → 0 | 4 → **6** |
| Huffman fixed-address driver | `$8000` | 3,781 → 3,781 | 0 → 0 | 3 → **5** |
| Huffman portable, leaf inlining disabled | `$2000` | 3,851 → 3,851 | 0 → 0 | 4 → **6** |

TN's new home is `PopUp::block0::block1::item` at `$E2/$E3`. Its routine code
shrinks from 280 to 274 bytes and its inline storage shrinks by two bytes.
All earlier named homes remain: `Ord`, `Items`, `FindItem`, `Strcpy`, `Instr`,
`Fnamecmp`, `Sort`, `Tag` and `PrintNonzeroDigit`. Temporary allocation is
unchanged: 32 allocated bytes across 13 routines, with reuse between routines.

TN has four supported helper sites: three multiplies and `Range`'s five-byte
SArgs capture. Huffman has 14 supported helper sites in each configuration.
`ReadBit` and `BuildTree` now have composed entry certificates. All three Huffman
images remain byte-identical to their baselines; temporary allocation remains
36 bytes across seven routines for portable and 44 for the driver.

Neither workload invokes the newly certified public SYS string/memory bindings.
Separate VM controls execute both cartridge and standalone implementations:
the `SCopy` caller saves three bytes, and the three memory-operation callers
save nine bytes combined. Their private caller homes use `$E0`. These are
control-program savings, not additional TN/Huffman savings.

## Remaining blockers and fallback

- `ReadBits` still lacks public LONGCARD result metadata and loses its otherwise
  supported parameter ABI. `ReadHeader` and `Decode` consequently lack complete
  descendant entry proofs. Helper availability does not repair that separate
  public ABI boundary.
- Source-home discovery still rejects some arithmetic/pointer caller bodies,
  and local-only source leaves remain outside parts of discovery. These gates
  were intentionally retained. `BuildTree` gaining a callee certificate does
  not make its own frame eligible for placement.
- `WriteByte` and `BuildTree` retain unknown indirect writes. In the driver,
  `Decode` also declines homes live across `WriteByte` for that reason.
- TN still has uncertified machine entries and unknown writes in larger call
  chains. `PopUp` can place its short-lived `item` even though its complete
  routine entry certificate is unavailable.
- Parameterized native wrappers containing library buffer writes remain
  conservative until argument-relative effects can be substituted at their
  callers. An explicit destination overlapping helper scratch also retains
  unknown returning writes.

The earlier layout fallback remains active. TN declines the expanded write
summary for `Pull` after temporary ZP remapping, then selects conservative
replay; the profiled compilation records six such fallbacks across its
materialization attempts. Each Huffman configuration declines `WriteByte`'s
expanded reads after remapping and selects conservative replay. The fallback
has not been removed or disguised as a contract success.

TN's plain, counted and detailed-trace outputs agree. Each Huffman traced output
also matches the ordinary public output path. The recorded times are descriptive,
not a controlled speed comparison: TN preparation/selection/emission took about
24.3 seconds in this optimized development build, with frontend work excluded.
Work counters and trace manifests are retained in the receipt.

## Validation

The [per-slice checks](../../MIR6502_RUNTIME_HELPER_CONTRACT_IMPLEMENTATION_PLAN.md#implementation-checks)
cover signatures, selected bindings, stale dependencies, SArgs capture,
arithmetic, fault paths, exposed/numeric buffers and both library providers.
Final qualification broadened within MIR6502:

| Check | Result |
| --- | --- |
| `cargo test --lib mir6502::` | 1,525 passed, six pre-existing failures, 15 ignored |
| Runtime-contract unit cases within that run | All 10 passed |
| `mir6502_call_abi` integration target | 11 passed |
| `cross_call_homes` VM target | 14 passed |
| `runtime_library_contracts` VM target | Three passed, both providers |
| `callee_facts` VM target | 10 passed, one pre-existing placement assertion failed |
| `huff_dec_mir6502 huff_dec_vectors_accept` | Three passed; 370 executions, LF/CRLF loader coverage |
| `tn_checked_arena` VM target | Two passed, one frozen-control test ignored; both runtimes, LF/CRLF |
| `tn_dispatch` VM target | Two passed; real PopUp, lookup/bar/save/restore, panel and dispatch paths |
| Combined TN/Huffman fact probe | Two passed, including independent fact controls |

The six unit failures and one VM failure were reproduced at `0a291ceb` in an
isolated worktree. No failing expectations were relaxed. The unit failures are
in constructor storage, pointer retention, three scalar-capture observation
cases and the expanded-entry handoff control. The VM failure is
`callee_facts_directional_reader_preserves_caller_and_pointee`: execution checks
pass, but the assertion expects every `KeepWriter` parameter inline when the
dead pointer is at `$E0`. [checks.json](checks.json) records exact test names,
counts and log hashes. The temporary baseline worktree was removed.

The TN targets also execute their existing classic optimized controls. This is
MIR6502 qualification with affected integration and VM targets, not a full
repository test run. No shared frontend/NIR contract was changed.

## Receipt and reproduction

- [baseline.json](baseline.json): compact starting homes, entry availability,
  routine sizes and image hashes. Huffman uses the matching
  [original audit](../6502-huffman-facts/README.md).
- [facts.json](facts.json): current named/temporary homes, per-routine full and
  normal-return effects, entry facts and helper/library invocation contracts.
- [qualification.json](qualification.json): before/after comparisons, source
  and image hashes, configurations, work counts, fallback events and trace
  equality checks.
- [checks.json](checks.json): final validation and baseline failure controls.

Run from the repository root:

```sh
python3 docs/benchmarks/6502-runtime-contracts/run.py
python3 docs/benchmarks/6502-runtime-contracts/summarize.py
```

The runner temporarily includes the existing diagnostic probes, runs only their
two ignored tests and removes the include in `finally`. Do not build or edit
`known_callees.rs` concurrently. It uses the existing shared target, two build
jobs, optimization level 1 and disabled debug information. The summarizer
retains the recorded starting baseline. Large images, listings, MIR and raw
traces remain under ignored `build/`; no diagnostic hook remains in production.
