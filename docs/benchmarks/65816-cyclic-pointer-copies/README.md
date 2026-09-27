# Native cyclic pointer copies

Captured three-byte pointer edges now break cycles with one reusable private
stack capture. Each pointer transfer uses overlapping A16 words at offsets zero
and one. A separate two-byte slot preserves the original accumulator, including
hidden B; the final bank-byte load restores the bytewise edge's A and N/Z.
Swaps, rotations, repeated sources and multiple independent cycles are supported.
Partial overlaps, constants and mixed-width edges retain their existing paths.
See the [emission contract](../../MIR65816_EMISSION_CONTRACT.md).

Allocation, verification and emission share the checked schedule and staging
requirements. Complete source, destination and staging extents are checked before
emission, including transient stack movement and staging overlap. All scratch
belongs to the current invocation. No public ABI, source-memory access, DP
reservation, push or call changes are introduced.

## Measured effect

The baseline is actionc `7c51b22e`, after native 24-bit argument packing. All 120
input hashes match the frozen Exec `622b139-dirty` workload: 631 routines, eight
task slots, shell, console/windows and MyDOS.

| Measurement | Before | After | Saved |
|---|---:|---:|---:|
| Frozen Exec compiler code, guards included | 389,743 B | 389,725 B | **18 B** |
| Frozen Exec compiler code, guard ranges subtracted | 317,491 B | 317,473 B | **18 B** |
| Optimized ExecList, guard ranges subtracted | 2,404 B | 2,386 B | **18 B** |
| Optimized FindName, guard range subtracted | 407 B | 389 B | **18 B** |
| FindName fixed frame and local stack peak | 28 B | 26 B | **2 B** |

Only `FindName` changes in the frozen Exec build. Its cyclic edge shrinks
**52 → 34 bytes**, with no branch-size changes. The frame uses an A-save word
and one pointer capture instead of two complete pointer captures. Incoming
argument displacements move down by two bytes; their public offsets, sizes and
alignment are unchanged. Temporary homes are unchanged.

All 2,676 guards retain their sizes, totalling 72,252 bytes. FindName's guard
amount follows its frame from 28 to 26 bytes; every other guard is unchanged.
Compiler initialized data remains 951 bytes. Reserved bank-zero capacity is
unchanged. This is a compiler size build; packaging and hosted qualification
were not rerun. Guard subtraction is not a separately compiled release image.

The gain is local: this cycle occurs only once in the frozen compiler output.
It is still useful in a loop: each executed FindName swap saves **22 cycles**.
Measured list-vector savings range from zero to 66 cycles, with no regressions.
Private stack byte reads/writes increase by three/two per swap because of the
overlapping words and full-A preservation, while instruction count and time
decrease. External-memory and DP traffic are unchanged. Raw ExecList output is
unchanged at 3,104 bytes including guards; optimized output is 2,926 bytes,
including unchanged 540-byte guards.

[Exec summary and hashes](exec-summary.json), [routine sizes](exec-routines.csv),
[changed MIR span](exec-spans.csv), [list sizes](list-sizes.csv),
[runtime deltas](lists/delta.csv), and [assembly](lists/actionc-optimized.lst)
retain the measurements.

## Focused validation

- Two planner tests cover all 125 simultaneous three-pointer assignments,
  independently executing the overlapping word transfers, plus invalid geometry
  and separate cycles across stack/DP homes.
- Fifteen pointer-selector, six staging-allocation and four pointer-coalescing
  unit tests pass. The cycle checks cover A8/A16 entry, stack/DP combinations,
  exact byte-255 limits, transient stack movement, missing/undersized/overlapping
  staging, invalid targets and rejection before emission. Allocation checks
  include swaps, rotations and three independent cycles sharing one capture.
- Thirty-four emission, o65 and reviewed state-boundary integration tests pass;
  the existing snapshot is unchanged.
- Twenty-five focused debug runtime tests pass across pointer edges, selective
  staging, word edges, replay and state tracking. The six pointer-edge tests
  also pass in release. An independent ca65 oracle compares complete registers,
  flags and non-staging memory with bytewise staging for swaps, rotations and
  cycle fan-out, both incoming accumulator widths and all supported flag states.
- The compiled cyclic backedge preserves simultaneous pointer values and full
  edge state. Raw/optimized o65 tests use two placements. The task test injects
  IRQ and NMI at every reached enabled Walk instruction in both task domains,
  including reentrant calls using the same cyclic routine.
- All 270 paired-mask list records pass in both host profiles. Debug/release
  measurements and actual LF/CRLF builds are identical.

The synthetic loop's constant-bearing entry still needs full byte staging;
its shared pool therefore retains two three-byte slots. Its cyclic backedge
uses only the first slot's A-save word and the second slot's pointer capture.
The dedicated allocation tests verify the compact `[2, 3]` requirement when no
other edge needs a wider pool.

[Validation provenance](validation.json) records source hashes, logs and focused
run manifests. Final backend and hosted Exec qualification were not run, as
requested.

The runtime commands were:

```sh
python3 tools/native65816-runtime-tests/qualify.py \
  --test pointer_edges --test selective_staging --test word_edges \
  --test replay --test state_tracking
python3 tools/native65816-runtime-tests/qualify.py --release \
  --test pointer_edges --test code_quality -- --include-ignored --nocapture
```

The release command also uses `A816_COMPARISON_MANIFEST` and
`A816_COMPARISON_RESULTS` for the Action-only manifest built by
`tools/compare65816/execlists.py`. The debug list run uses the same manifest and
`--test code_quality -- --ignored --nocapture`. `report_pointer_micro.py`
checks and archives both sets of results against the native-pointer-arguments
baseline.
