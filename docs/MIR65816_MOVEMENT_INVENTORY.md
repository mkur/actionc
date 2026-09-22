# Native 65816 remaining-copy and reload inventory

Measured on 2026-09-22 against selective-staging compiler `8af3541`, qualified
by `e263872`. This is an inventory and diagnostic tooling slice: compiler
allocation, emission, ABI, stack guards and generated images are unchanged.
The older [copy inventory](MIR65816_COPY_INVENTORY.md) stays frozen.

The [machine-readable inventory](benchmarks/65816-movement-inventory/inventory.json)
covers all 28 raw/optimized Action builds, 30 counted routine bodies and 132
Action vector records in the 14-kernel corpus. Frequencies below sum vectors
**per incoming I state**. Saved debug/release and paired-I measurements agree;
static sites are counted once per build. The existing 264-record comparison,
including vbcc, remains intact.

## Findings

| Observation | Static sites/assignments | Executions |
| --- | ---: | ---: |
| Nonempty edge assignments | 10 | 254 |
| Existing physical edge self-copies | 0 | 0 |
| Compatible private temp pairs on edges | 2 | 12 |
| Interfering private temp pairs on edges | 5 | 226 |
| Immediate edge sources | 3 | 16 |
| Word-width stack LDA instructions | 172 | 3,324 |
| Additional forwarding with current temp producer/consumer classes | 0 | 0 |
| Redundant frame-object word load | 1 | 48 |
| Redundant incoming-parameter word loads | 2 | 28 |

The six nonempty edges comprise four direct single-word edges, one acyclic
multi-word edge and one selectively staged cyclic edge. There are another 34
empty edges, for which no copy execution count is invented. The reload scanner
excludes 112 byte-width stack loads: forwarding a partial accumulator lane needs
separate proofs. It does not propose caching external/volatile/pointer memory or
keeping values across calls, helpers or joins.

### Reload candidates

All three candidates require adding frame/parameter load consumers to the
forwarding policy. Merely extending the lifetime of the current temp-forwarding
permission yields no additional qualified site in this corpus.

| Kernel / mode | Reload PC | Source | Executions across vectors | Conditional saving |
| --- | --- | --- | ---: | --- |
| `loop_rotation`, raw | $010034 | Incoming word at S+$16 | 6 | 2 static bytes; 30 cycles, 12 stack-byte reads |
| `loop_rotation`, optimized | $010056 | Frame word at S+$02 | 48 | 2 static bytes; 240 cycles, 96 stack-byte reads |
| `recursive_sum`, raw | $01005A | Incoming word at S+$0C | 22 | 2 static bytes; 110 cycles, 44 stack-byte reads |

These are conditional instruction-removal forecasts, not optimized outputs.
Each LDA costs five cycles, reads two stack bytes and changes no register other
than PC at every observed execution. Full A and N/Z already match its source;
C/V and other status bits remain unchanged. Removing only these loads would
save 76 instructions, 380 cycles and 152 stack-byte reads across all vectors per
I state. Stores, frames, guard costs and incoming ABI offsets would remain.

For input 13, the individual forecasts are:

| Kernel / mode | Bytes before → forecast | Cycles before → forecast | Stack peak |
| --- | ---: | ---: | ---: |
| `loop_rotation`, raw | 172 → 170 | 1,226 → 1,221 | 18 |
| `loop_rotation`, optimized | 140 → 138 | 956 → 916 | 16 |
| `recursive_sum`, raw | 208 → 206 | 3,467 → 3,402 | 190 |

The optimized rotation's window is particularly small:

```asm
LDA $0A,S
STA $02,S
LDA $02,S       ; candidate: A and N/Z already describe this full word
STA $0C,S
```

The raw rotation reloads an unchanged incoming parameter after a disjoint frame
store. Raw recursion reloads an incoming parameter after retaining its first
temporary capture. The inventory records every instruction of each proof window,
per-vector frequency and required proof. Any implementation must preserve source
memory ordering, authoritative parameter homes, full A/N/Z, and interrupt
restoration. Start conservatively around address-taken objects and aliases;
a measured instruction pattern alone is not a general semantic permission.

### Coalescing candidates

Only the optimized rotation's initialization has noninterfering edge pairs:
`t0 → t16` and `t2 → t17`, each executed once per vector. One-at-a-time probes
show why pair compatibility alone is insufficient: the second pair's existing
homes conflict with other live temps in either direction.

A combined diagnostic recoloring passes the existing frame verifier:

- Move `t0` from S+$06 to `t16`'s S+$0A.
- Move `t2` from S+$08 to `t17`'s S+$06.

These hypothetical homes are passed only to verification, never emission. Both
initialization assignments then become physical self-copies. Their isolated
copy-removal ceiling is eight bytes, four instructions, 20 cycles, four stack-byte
reads and four writes per call; 120 cycles across six vectors. The final immediate
assignment still establishes A/N/Z. The accepted probe retains a 16-byte frame.
This is not a complete allocation/emission forecast and must not be added to the
reload forecasts without compiling and measuring the combined change.

The five other temp pairs interfere under current closed-operation liveness.
The cyclic rotation's live words must remain distinct. The `sum_loop` and
`byte_sum` result/input pairs coexist during their arithmetic operation. Removing
those conflicts would require a separate, finer-grained allocation contract;
globally relaxing the interference rule is not justified by this inventory.

## Recommended next slice

Plan **native word forwarding from a direct frame store to a following load**.
The optimized rotation provides a reached, simple first case with a 40-cycle
saving at input 13. Retain all stores/homes and guard behavior. Give the new
consumer class its own typed witness, exact-byte checker and IRQ/NMI coverage.
Repeated incoming-parameter loads can be a subsequent extension or included only
if their distinct home/effect proof remains small.

Keep edge-home coalescing separate. It has two compatible initialization pairs
and a smaller dynamic benefit here. Neither candidate improves `sum_loop(13)`:
it remains 120 bytes / 1,212 cycles / 12 stack bytes. Scalar DP allocation is
still the later candidate for reducing its repeated memory traffic.

## Method and validation

The new [typed exporter](../tests/mir65816_movement_inventory.rs) recompiles each
corpus source, verifies MIR and requires complete serialized-image equality with
the qualified saved artifact, including uncounted routines. It repeats actual
source parsing/compilation for LF and CRLF. It exports IDs, byte homes, complete
operation spans, every machine label, CFG uses/definitions, typed edge transfers
and bounded diagnostic frame-verifier probes. It makes no compiler edits.

The independent [inventory tool](../tools/compare65816/inventory_movements.py):

- Checks all qualification, input, measurement and artifact hashes before reusing
  counts. It reconstructs direct, acyclic, selective and fallback copy bytes and
  reconciles typed edges with the saved runtime copy counters.
- Reconstructs conservative closed-operation live points, including dead outputs,
  calls, branch arguments, block parameters and successor live-ins. It reports
  interference witnesses, third-party home conflicts and verifier results for
  bounded individual/combined recolorings (at most eight temp pairs per edge).
- Tracks exact private byte identities, A16 and N/Z through reviewed instructions.
  Partial writes update byte identities; CMP invalidates the N/Z relation.
  Labels, calls/helpers, volatile/alias-sensitive accesses, width/S changes and
  unreviewed effects break proofs. It lists all word stack loads, including
  noncandidates, and does not treat existing omitted loads as new opportunities.

The [VM observer](../tools/native65816-runtime-tests/tests/movement_inventory.rs)
executes original saved bytes without using claims to control execution. Across
264 Action executions per host (132 vectors × two I states), it reproduces every
baseline instruction count, cycle count, result and expected source-memory
outcome. At each claimed load it checks the exact proof-window bytes, A/home and
N/Z equality, then executes the LDA and checks complete registers, five cycles
and the two stack reads. Debug/release observations match exactly.

The exporter test passes, including all 56 LF/CRLF compilations. All 46 comparison
tool tests pass, covering reordering/final-A reloads, selective capture maps,
byte fallback, partial writes, flag changes, calls, aliases, joins, mode changes
and coalescing conflicts. The inventory reproduces with `--check`.
[Validation provenance](benchmarks/65816-movement-inventory/validation.json)
records tool hashes, native manifests and observations. No embedded fixture or
compiler contract changed, so the full root/NIR/native suites were not repeated.
The known optimized vbcc `unlink` failure remains visible in the reused baseline.

## Reproduce

```sh
A816_COMPARISON_MANIFEST="$PWD/target/selective-staging-after/manifest.json" \
A816_MOVEMENT_FACTS="$PWD/target/movement-inventory-facts.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --test mir65816_movement_inventory -- --ignored
python3 -B tools/compare65816/inventory_movements.py target/selective-staging-after \
  --facts target/movement-inventory-facts.json \
  --output docs/benchmarks/65816-movement-inventory/inventory.json --check
A816_COMPARISON_MANIFEST="$PWD/target/selective-staging-after/manifest.json" \
A816_MOVEMENT_INVENTORY="$PWD/docs/benchmarks/65816-movement-inventory/inventory.json" \
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  python3 -B tools/native65816-runtime-tests/qualify.py \
  --test movement_inventory -- --ignored
```

Run the observer again with `--release` and the same environment. Omit `--check`
only when deliberately publishing a new inventory; leave historical baselines
unchanged. Both readers reject evidence that no longer matches the saved compiler
artifacts. The full JSON includes rejected candidates and per-vector forecasts.
