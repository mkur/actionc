# Argument setup A to X forwarding results

Measured 2026-11-11 following the
[four-slice plan](../../MIR6502_ARGUMENT_X_FORWARDING_PLAN.md).
[baseline.json](baseline.json) freezes the preceding compiler and eight image
controls. [results.json](results.json) records source/image hashes, final
sizes, storage, site counts, timings and execution checks. All eight
feature-off images match their frozen hashes.

`Mir6502Config::optimized()` enables `enable_argument_x_forwarding`;
`Mir6502Config::default()` keeps it disabled. After a byte store from A, the
selector inserts TAX and removes a later reload of the same byte into X.
The intervening setup stays in order. A zero-page reload saves one byte and
one cycle; an absolute reload saves two bytes and two cycles. The stored
home, all other stores, parameter storage and public ABI remain intact.

## Sizes and execution

Only this feature changes within each pair. Both settings use modern MIR6502,
optimized NIR and entry-reachable linking. Sizes include XEX framing. Neutral
fixtures use `$3000` and `$41C7`; TN/TNDBG use `$2C00`.

| Program | Runtime | Before | After | Saved |
| --- | --- | ---: | ---: | ---: |
| New neutral, either origin | cartridge | 360 | 356 | 4 |
| New neutral, either origin | standalone | 442 | 438 | 4 |
| TN | cartridge | 9,862 | 9,858 | 4 |
| TN | standalone | 9,987 | 9,983 | 4 |
| TNDBG | cartridge | 12,958 | 12,954 | 4 |
| TNDBG | standalone | 13,083 | 13,079 | 4 |
| Existing pointer load sharing fixture | cartridge | 710 | 704 | 6 |
| Existing pointer increment fixture | cartridge | 639 | 634 | 5 |
| Predicate snapshots | cartridge | 640 | 638 | 2 |

The neutral fixture covers fixed staging, an ordinary BYTE parameter and a
private zero-page home. `Forward`, `ForwardByte` and `ForwardPrivate` shrink
84→83, 11→9 and 47→46 bytes. Sample calls take 342→341, 291→289 and
444→443 cycles, including the observing callee and return. The earlier
restricted-forwarding control `Plain` stays 40 bytes and 429 cycles. Both
runtimes and origins agree.

TN saves one byte each in `MoveMenuBar`, `Strcat`, `Draw` and `TagAll`.
TNDBG qualifies the same four routines. `Strcat` is 114→113 bytes and
a call with `s=$7000`, `t=$7200` and lengths 3/8 takes 356→355 cycles in
both variants/runtimes. Ordered indirect reads, retained ABI/pointer stores
and the 20-byte post-call reconstruction remain. Parameter-home overwrite
probes still follow the updated pointers. The historical 51-byte C control
leaves a 62-byte whole-body difference with distinct ABI and alias contracts;
that difference does not establish further removable bytes.

Frames, source-storage capacities, reserved lengths and occupied zero page
are unchanged. Code-relative data and reserved ranges can relocate. The
zero-page census unions declared, allocated and mapped cells, excluding
implicit runtime-only helper scratch.

## Applicability and limits

| Input | Typed candidates | Applied | Image saving |
| --- | ---: | ---: | ---: |
| New neutral, either origin | 4 | 3 | 4 |
| TN | 9 | 4 | 4 |
| TNDBG | 14 | 4 | 4 |
| Existing pointer load sharing fixture | 7 | 6 | 6 |
| Existing pointer increment fixture | 5 | 5 | 5 |
| Predicate snapshots | 1 | 1 | 2 |
| Graphics (`fedora`) | 1 | 0 | 0 |
| Direct word arithmetic | 0 | 0 | 0 |
| Indexed byte arguments | 0 | 0 | 0 |
| Scalar/predicate captures | 0 | 0 | 0 |
| Array-parameter string loop | 0 | 0 | 0 |
| Resident string calls | 0 | 0 | 0 |

Counts apply separately to each runtime. Fourteen of 28 runtime/origin pairs
shrink and fourteen are byte-identical. Across these images, 52 applications
save 58 bytes. Each removes one direct X load and no stores; total MIR
operation count stays equal because TAX replaces the load. Different first
argument producers and `q+0`, `q+2`, `q-1` or offset consumers can qualify
through the same proof.

TN rejects three candidates for unavailable call-entry flag facts and two for
unsupported gap operations. TNDBG rejects seven for call flags, two for the
gap and one for call shape. Its `Dbg` candidates remain because complete
flag-free entry facts are unavailable. Neutral and graphics controls reject X inputs; the
older pointer fixture also contains a physical-alias rejection. There are no
late cost or exit-fact rejections in the measured corpus.

The proof is bounded to 64 operations through one same-block direct call,
with at most three simple tail operations. It accepts ordinary parameter/local
or private homes and backend staging `$A0..$A4`. The closed setup vocabulary
covers byte A/Y loads/stores, immediate arithmetic and fixed-pair Y indirect
byte loads. Indexed-X inputs, X clobbers, unknown writes, wrapping pointer
pairs, helpers, machine blocks and intermediate calls retain their fallback.
Physical writes must leave the stored byte unchanged. Calls use the canonical
materialized byte ABI with X as the second lane.

Inserted TAX and removed LDX each require a separate N/Z proof. A retained
tail definition restores exact flags before machine-call observation. Without
restoration, known scalar entry facts must establish that incoming flags are
unused and need not be preserved. Entry liveness checks every CFG path,
conservatively treating nested calls/helpers/barriers as flag observations;
a scalar signature and routine effects alone are insufficient. This deliberately leaves some otherwise
plausible sites unselected. C/V, A/Y and stack state retain their values.
Shared typed witnesses are reconstructed by the rewrite driver using its
snapshot and supplied physical map. Final costing replays accepted routines
against the proposed layout, preserves published exits and requires a strictly
smaller XEX without code/home growth.

A cheap shape scan skips absent candidates. Each transaction decreases the
original direct X-load rank, despite equal operation counts. An accepted batch
uses seven additional top-level emissions: two physical maps, three routine/
home costs and two XEX costs. Proof-only rejections use one map emission;
inputs without typed candidates use none. It performs no extra full materializations or
per-site emission trials. Unknown cost or failed replay retains the baseline.
Single release-profile materialization samples are 56.1→59.8 ms for neutral
cartridge, 11.82→8.83 s for TN and 24.91→25.13 s for TNDBG. These single
samples under concurrent qualification load do not establish compiler-speed
changes.

## Validation and reproduction

Eleven focused tests include 24,576 emitted fragment executions over all 256
forwarded byte values, both carry inputs, word boundaries and capture/pointer/
source aliases at two origins. An independent host oracle checks restored
A/X/Y/C/N/Z/V/SP. Three VM tests cover 1,536 calls across raw/optimized NIR,
both runtimes/origins and actual LF/CRLF parsing, plus 104 alias calls and
64 calls across all eight preceding-feature settings. Retained writes and
ordered single indirect reads agree. Exact flags are compared after tail
restoration; the flag-free BYTE callee is checked through its nested restored
observer. Both feature settings reproduce the measured neutral images.

Eight exact TN/TNDBG images pass 80 counted-string and 16 post-call parameter
alias cases, including same-buffer and overlapping copies. Positive source
lengths retain the existing string oracle's scope. At qualification the default
TN/TNDBG images reproduced these files and passed panel/dispatch execution.
Current default images include the later [immediate bitwise forwarding](../6502-argument-x-bitwise/README.md).

Backend qualification passes 1,217 MIR6502 unit tests, 105 integration tests
in 18 targets, 169 emission/load fixtures and 33 tests in ten affected VM
targets, with two TN dispatch tests separately. Eight LF/CRLF listing
reassemblies and relocations pass
with MADS, as do scoped arithmetic/indexed/indirect-field runtime assertions.
The frozen Strcat static and VM audits retain their historical images. The
previously documented indexed-byte selection-count discrepancy remains
separate from its passing runtime assertion. No shared NIR or semantic
contract changed. Exact check counts are recorded in `results.json`.

The opt-in exporter writes XEX, assembly, MIR, entry/parameter maps and full
frame/storage facts under ignored `build/argument-x-forwarding/measured/`:

```sh
CARGO_INCREMENTAL=0 cargo test --release --locked --lib argument_x_forwarding_measure_corpus -- --ignored --nocapture
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test argument_x_forwarding_measurements -- --ignored --nocapture
cargo test --locked --test mir6502_argument_x_forwarding -- --ignored --nocapture
```

The last command requires MADS on PATH or `ACTIONC_MADS`. Set
`ACTIONC_ARGUMENT_X_MEASURED_IMAGES` to the measured directory's absolute path
when running `argument_x_forwarding` to require historical neutral image
equality. Current default `tn_dispatch` uses
`ACTIONC_ARGUMENT_X_BITWISE_MEASURED_IMAGES` and the later report. Set `ACTIONC_ARGUMENT_X_BASELINE` to the frozen `before` directory
for the neutral baseline check. Earlier exporters and baseline-sensitive
helpers explicitly disable later features, including precise entry flags;
their manifests and byte-role
reports remain reproducible. The plan lists full scoped qualification commands.
