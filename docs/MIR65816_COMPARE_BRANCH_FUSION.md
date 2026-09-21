# Native 65816 compare-to-branch fusion

Completed on 2026-09-21. The emitter combines a final eligible word Compare and
its sole consuming Branch into one CMP/conditional dispatch. Both raw and
optimized compilation use this selection. The [plan](MIR65816_COMPARE_BRANCH_FUSION_PLAN.md)
and [emission contract](MIR65816_EMISSION_CONTRACT.md) describe the scope and
invariants; the [qualification record](abi/action65816-compare-branch-qualification.json)
binds the results to source, tools, VM and artifact hashes.

## Selection and safety

A routine-wide use proof distinguishes Branch conditions from every other input,
including both edge argument lists, successor uses, returns, indirect calls and
address operands. Any other use disqualifies fusion, as does an intervening
operation. The selector shares checked operands and predicate selection with
ordinary word comparisons: unsigned Eq/Ne/Lt/Le/Gt/Ge and signed Eq/Ne. Signed
ordering, byte/wide comparisons and unsupported homes retain existing emission.

CMP flags are consumed immediately, before either edge stages its parallel
copies. True and false edge trampolines retain their typed JML fixups, copies
and mode restoration, including equal successor targets with different arguments.
Calls and original volatile/aliased memory accesses stay in place. Only captured
stack words and immediates participate in the fused operation. No DP scratch,
helper, push, X/Y allocation or cross-operation register lifetime is introduced.

The Boolean home remains validated and reserved but is not written when no
Boolean consumer remains. Materialized Booleans still receive exactly 0 or 1.
Physical ABI v1, image v3, o65 profile, frame allocation/storage maps, guard costs,
stack depth and Exec816's pin are unchanged. This slice reduces traffic, not
allocated stack space.

## Measured results

The immutable baseline is the [post-word-comparison snapshot](benchmarks/65816-word-comparisons/after/tables.md),
emitted at `51e5bc7` and qualified at `4d54b81`. Before implementation, 224 saved
file hashes and all 264 matching debug/release records were rechecked. Baseline
semantic coverage and the use proof were committed as `e4dbdd1`; fused selection
and focused machine-code checks were committed as `4687170`.

The [new snapshot](benchmarks/65816-compare-branch/after/tables.md) and
[complete delta](benchmarks/65816-compare-branch/delta.md) cover 14 paired kernels,
66 vectors, both target modes and both incoming I states in both host builds.
All cells below include guards and RTL; stack depth is measured below entry S.

| Kernel / input / mode | Code bytes before → after | VM cycles before → after | Stack reads before → after | Stack writes before → after | Stack depth |
| --- | ---: | ---: | ---: | ---: | ---: |
| maximum(13,41), either | 146 → 116 | 149 → 117 | 16 → 15 | 7 → 6 | 6 |
| sum loop(13), optimized | 213 → 183 | 2,465 → 2,030 | 287 → 273 | 230 → 216 | 16 |
| sum loop(13), raw | 218 → 188 | 2,596 → 2,161 | 317 → 303 | 260 → 246 | 14 |
| recursive sum(13), optimized | 247 → 217 | 3,819 → 3,372 | 268 → 254 | 250 → 236 | 190 |

The forecasts match exactly: 30 fewer bytes per selected site, 31 fewer cycles
for true and 32 for false. Six kernels fuse one site each in both modes: maximum,
loop rotation, sum loop, recursive sum, byte sum and forward copy. Every executed
fusion removes exactly one Boolean stack write and one reload. All 58
[predeclared per-vector counts](benchmarks/65816-compare-branch/fused-branch-counts.json)
match reached, decoded sequences; all other records retain their previous stack
traffic. DP accesses and touched offsets are unchanged, including zero traffic
in the representative kernels above. No Action correctness, size or cycle
regression occurs, and every routine's storage contract remains identical.

Both external comparison commands still report the existing optimized vbcc
unlink failure. All measurements are saved, debug/release results agree, and
vbcc records are identical to baseline. The failure remains visible without an
expected-success exemption.

## Execution and interruption coverage

Six compare/branch tests cover all signed/unsigned relations and byte/wide
fallbacks across boundary pairs; LF/CRLF image equality; reused conditions,
edge arguments and intervening operations; volatile, aliased and bank-crossing
inputs across direct/indirect clobbering calls; same-target edges and parallel
backedge swaps; decoded source reads, no comparison writes/DP access, and
separate edge-copy traffic. The decoder rejects malformed or truncated edge
sequences and wrong accumulator modes. Existing materialized-comparison byte
encodings, traffic and canaries remain tested.

General preemption reaches 2,275 raw and 2,117 optimized enabled instruction
addresses, including three fused comparison windows, seven arithmetic windows
and seven return tails. The targeted fused probe covers 152 task/PC sites per
mode, six stack/immediate comparison windows and all 24 window/truth/task
combinations immediately after CMP. Nonempty edges return full word arguments.
IRQ is armed at an instruction boundary and sampled at instruction completion;
a separate uninterrupted CPU step supplies the expected register state. The
probe checks every restored register, including live A/P, before continuing and
checking final outputs and guards. Both seeded IRQ/NMI schedules pass.

The separate materialized-comparison probe retains its 96 task/PC sites and 16
predicate/truth/task combinations. Relocated o65 execution adds eight fused
conditional consumers at each of two code/data/BSS placements, with both target
modes, both incoming I states and boundary inputs. The original o65 materialized
Boolean probe is retained.

Full qualification passes **67 native tests in debug and release**. Compiler
checks pass 28 native library tests and 58 integration/CLI tests; decoder and
delta tooling pass five and six Python tests respectively. The code-quality test
is ignored in the ordinary native suite and was run separately in both builds.
See the qualification record for exact manifests and identical saved artifacts.
No NIR contract changed, so repository-wide NIR sweeps and the full root suite
were outside this native emission slice.

## Remaining scope

Removing the now-unused Boolean reservation would require a separate allocation
and storage-map change. Broader flag lifetimes, byte/wide fusion, signed ordering
and empty-edge cleanup remain independent follow-up work. Qualification executes
serialized machine code on the corrected pinned VM; board interrupt/startup
acceptance remains a separate integration boundary.
