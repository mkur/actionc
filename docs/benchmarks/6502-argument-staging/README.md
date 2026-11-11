# Direct-call argument staging: measured results

Measured 2026-11-11 against the compiler and inputs frozen in
[baseline.json](baseline.json). The implementation follows the
[design note](../../MIR6502_ARGUMENT_STAGING_DESIGN.md) and
[four-slice plan](../../MIR6502_ARGUMENT_STAGING_IMPLEMENTATION_PLAN.md).
[results.json](results.json) records image/source hashes, code/storage
extents, declared/allocated zero page, site counts, timings and cycles. All
eight feature-off neutral/TN/TNDBG images match their frozen controls exactly.

The selected shape is a direct scalar WORD/WORD/BYTE call whose first word
is ordinary direct storage plus or minus a constant. After the existing
materializer captures the arithmetic result, the new schedule uses `TAX`
beside the high-byte store and removes a later `LDX` of that capture. Typed
proofs check the complete arithmetic, intervening setup, live registers/flags,
canonical ABI, physical aliases and final placement. Capture stores and home
allocations remain intact. SemIR, NIR and the public ABI are unchanged.

## Sizes and execution

Only `enable_argument_staging_reduction` changes between each control pair.
Both use modern MIR6502, optimized NIR and entry-reachable linking. Sizes
include XEX framing. Neutral origins are `$3000` and `$41C7`; TN uses `$2C00`.

| Program | Runtime | Before | After | Saved |
| --- | --- | ---: | ---: | ---: |
| Neutral fixture, either origin | cartridge | 336 | 334 | 2 |
| Neutral fixture, either origin | standalone | 418 | 416 | 2 |
| TN | cartridge | 9,873 | 9,873 | 0 |
| TN | standalone | 9,998 | 9,998 | 0 |
| TNDBG | cartridge | 12,969 | 12,969 | 0 |
| TNDBG | standalone | 13,094 | 13,094 | 0 |

Each of `ForwardAdd` and `ForwardSub` shrinks from 41 to 40 bytes and removes
one executed private-memory load. Each sample call takes 430→429 cycles,
including the common wrapper, argument observer's real callee and return.
The representative inputs are first word `$00FF`, second word `$8000`, tag
128; both runtimes and origins agree. The existing four-byte arithmetic and
plain five-byte controls stay at 33/28 bytes and 111/412 cycles respectively.
All stores, ordinary home capacities and declared/allocated zero-page bytes
are unchanged. Code-relative ordinary storage may relocate with shorter code;
the final physical proof is replayed against that placement.

TN's `Strcat` remains 125 bytes. Its first word includes `s^`, its second
word is another computation, and its tag is `t^`: these do not satisfy the
ordinary direct-source template. Its `MovePage` target is also a machine
routine rather than an admitted scalar signature. The first-release gate
rejects the producer shape before proposing a schedule. Supporting those
indirect reads and machine-call setup requires a separate extension. This
change does not close the historical 125-versus-51-byte whole-body gap.

The [Strcat byte and cycle breakdown](STRCAT_BYTE_BREAKDOWN.md) accounts for
all 125 bytes and the 74-byte C-control difference. It also executes parameter
aliases that explain why post-call pointer reloads need stronger proofs.

## Applicability and cost

The corpus includes neutral arithmetic, TN/TNDBG, existing direct word
arithmetic and indexed-byte arguments, scalar/predicate captures and their
snapshot regressions, an array-parameter string loop, graphics (`fedora`) and
resident string-runtime calls. The committed results distinguish pre-home
admission from final schedule candidates and applications. Shape mismatches
are not counted as late alias or cost failures.

| Input | Pre-home calls | Admitted | Final candidates | Applied |
| --- | ---: | ---: | ---: | ---: |
| Neutral, either origin | 8 | 2 | 2 | 2 |
| TN | 347 | 0 | 0 | 0 |
| TNDBG | 529 | 0 | 0 | 0 |
| Existing word arithmetic | 12 | 0 | 0 | 0 |
| Indexed byte arguments | 2 | 0 | 0 | 0 |
| Scalar/predicate captures | 4 | 0 | 0 | 0 |
| Predicate snapshots | 14 | 0 | 0 | 0 |
| Array-parameter string loop | 1 | 0 | 0 | 0 |
| Graphics (`fedora`) | 64 | 0 | 0 | 0 |
| Resident string calls | 3 | 0 | 0 | 0 |

Counts apply independently to each runtime. Across all 22 runtime/origin
pairs, four neutral pairs improve; every other image remains byte-identical.
The template is a small reusable primitive, with narrow observed applicability.

The selector evaluates one template and the existing fallback, with at most
five argument bytes and 64 producer/setup operations. Each accepted rewrite
removes one high-byte reload; snapshots are rebuilt after each transaction.
It performs no additional whole-program materialization. A selected batch
uses at most seven emission passes for physical maps, existing code/home
costs, replay and exact final XEX costs. Those passes cover the batch rather
than running per call site. Unsupported programs still pay the bounded
pre-home admission check. Timing samples include normal materialization and
reporting and are single observations, not a compilation-speed benchmark.
The neutral cartridge sample at `$3000` takes 31.6→33.5 ms. TN cartridge takes
8.62→8.29 s and TNDBG cartridge 24.60→26.63 s under concurrent qualification
load; these noisy pairs cannot establish a compiler-speed change.

The zero-page census covers declared fixed cells, allocated homes and mapped
source storage. It does not claim to inventory every implicit runtime helper
scratch byte. Frame equality and emitted-image comparisons separately guard
against introducing storage through this extension.

## Validation and reproduction

Focused coverage includes three pre-home and eight post-home proof tests.
Emitted arithmetic fragments execute all 65,536 first-word values for both
operations with feature-off/on (262,144 executions). Three VM tests check
entry argument bytes, flags, stack depth, watched writes, changing source
homes and callee pointer aliases. The main oracle executes 6,144 calls with
raw/optimized NIR, both runtimes, both origins and actual LF/CRLF sources.

Final backend checks pass 1,182 MIR6502 unit tests, 102 integration tests across
all 15 MIR6502 targets and 169 fixtures through emission/load generation. Eight
affected VM targets pass 27 tests, plus the opt-in measured-image string oracle
and three focused arithmetic/indexed/indirect-field runtime assertions. The
neutral and TN cartridge VM tests require exact equality with measured images;
the string oracle also executes both TN variants in both runtimes, for 80
cases with positive source lengths. `MovePage`'s existing zero-length assembly
behavior is unchanged and excluded from that counted-string oracle.

One existing check is stale: `run-indexed-byte-fixed-action-args-vm.sh` expects
six `indexed-byte-direct-fixed-action-call-arg` selections but reports zero.
The pinned baseline reports the same zero count, and feature-off/on indexed
images are identical. Its underlying
`indexed_byte_fixed_action_args_execute_through_the_vm_library` assertion
passes when run directly. The direct-word and indirect-field scripts pass.

The optional corpus measurement writes images, MIR, entry tables and detailed
code/storage metrics under ignored `build/argument-staging/measured/`:

```sh
cargo test --release --locked --lib argument_staging_measure_corpus -- --ignored --nocapture
```

The historical corpus exporter explicitly disables later pointer-byte increment
fusion, pointer load sharing and argument X forwarding, keeping the frozen
controls reproducible. After measuring, set `ACTIONC_ARGUMENT_MEASURED_IMAGES`
to that directory's absolute path when
running the VM `argument_staging` target. Its independent execution test also
requires exact agreement with the measured neutral images. The opt-in string
oracle below and `strcat_cost_breakdown` execute the frozen TN images directly.
Current default TN execution uses the
[callee entry flag measurements](../6502-callee-entry-flags/README.md)
and `ACTIONC_CALLEE_ENTRY_FLAGS_MEASURED_IMAGES` for `tn_dispatch`. The plan lists the
full affected-backend qualification commands.

```sh
cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test argument_staging_measurements -- --ignored --nocapture
```
