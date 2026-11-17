# Huffman compiler facts and zero-page placement audit

Audited 2026-11-17 at `0a291ceb`, after the nine TN fact repairs. This audit
examines the eight Huffman kernel routines and their generated helpers using
the actual selected MIR6502 executable. No production compiler behavior changed.
External cartridge helpers were consumed through their existing contracts;
the emitted 32-bit shift helper bodies were also inspected directly.

**No incorrect positive fact or execution failure was found in this scope.**
The audit does expose substantial precision and coverage limits: ordinary
Action! routines lose entry proofs when lowering introduces runtime helpers;
a LONGCARD result suppresses otherwise supported parameter ABI information;
and several source-home discovery rules reject simple candidates before
allocation. Both audited programs have **zero named parameter/local ZP homes**.
Temporary allocation still uses ZP extensively.

## Programs and checks

The [portable program](../../../fixtures/runtime/tacle/huff_dec/huff_dec.act)
owns ordinary compiler-allocated buffers. The existing
[6502 driver](../../../tools/vm-runtime-tests/fixtures/huff_dec_driver.act)
uses fixed host addresses. Both include the same
[kernel](../../../fixtures/runtime/tacle/huff_dec/kernel.inc) and record types.
The portable program additionally contains the layout-query procedure used by
other backends, so their total sizes are not an optimization A/B comparison.

| Configuration | Origin | Packaged bytes | Routine instruction bytes | Named inline home bytes | Named ZP bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| Portable, optimized ActionCart | `$2000` | 3,851 | 3,700 | 33 | 0 |
| 6502 driver, optimized ActionCart | `$8000` | 3,781 | 3,671 | 34 | 0 |
| Portable, small-leaf inlining disabled | `$2000` | 3,851 | 3,700 | 33 | 0 |

Disabling small-leaf inlining leaves the portable image byte-identical. In
both layouts the parameter-home selector proposes no placement trial, and its
input/output image size is unchanged. The limits below are not cost rejections.
Packaged size excludes uninitialized buffers; it is not total RAM consumption.

The existing MIR6502 cartridge and standalone VM tests passed **185 C-reference
cases each, 370 executions total**. They compare control state, decoded output,
the complete code table and tree pool, unchanged inputs and guards throughout
`$0600–$7FFF`. Cartridge runs use LF and standalone runs use CRLF through the
actual source/include loader. The vector-format test also passed. Classic and
unrelated backend suites were not run. Portable storage was compiled and
inspected; the fixed-address driver supplied the executed workload.

## Selected source homes and temporary ZP

The following is the portable program. All listed named homes are inline;
register arguments and return slots are not persistent source homes.

| Routine | Named homes remaining | Inline bytes | Temporary ZP allocations |
| --- | --- | ---: | --- |
| `EndOfData` | None | 0 | None |
| `ReadByte` | `value` | 1 | None |
| `WriteByte` | None | 0 | `$E0` |
| `ReadBit` | None; three inline spill bytes remain | 0 | `$E0–$E1` |
| `ReadBits` | `n`, four-byte `value`, `i` | 8 | `$E0–$E6` |
| `ReadHeader` | `table`, `i`, `j`, `byteIndex` | 8 | `$E0–$EF` |
| `BuildTree` | `table`, `pool`, `j`, `top`, `root`, `current` | 12 | `$E0–$E6` |
| `Decode` | `root`, `current` | 4 | `$E0` |

`BuildTree.i` no longer has a named final storage entry. The table does not
claim a particular source identity for an anonymous spill. Additional inline
spill cells are recorded in [facts.json](facts.json).

In the fixed-address driver, `WriteByte.value` additionally remains inline.
Temporary usage rises to `$E0–$E2` in `WriteByte`, `$E0–$E5` in `ReadBit`, and
`$E0–$E8` in `ReadBits`; the other listed allocations match. These counts exclude
ABI/helper scratch such as `$A0–$A3`, `$AC–$AF`, `$82–$87` and `$C0–$C7`.
Sibling routines reuse addresses. `ReadHeader` already occupies all 16 bytes
of the general `$E0–$EF` temporary pool: promoting source homes could displace
temporaries, so an instruction-count estimate is not a measured saving.

## Fact audit

| Routine | Composed entry | Guaranteed result bytes | Reads / writes |
| --- | --- | --- | --- |
| `EndOfData` | No incoming register, flag or fixed-byte requirement | `$A0` | Exact / exact |
| `ReadByte` | No incoming register, flag or fixed-byte requirement | `$A0` | Unknown / exact |
| `WriteByte` | A supplies its byte argument; no extra input | None, as declared void | Exact / Unknown |
| `ReadBit` | Unavailable: runtime-helper operation | Not certified | Unknown / exact |
| `ReadBits` | Unavailable: helpers; direct ABI also unavailable | Not certified | Unknown / exact |
| `ReadHeader` | Unavailable: a call to `ReadBits` has unknown direct ABI | Not certified | Unknown / Unknown |
| `BuildTree` | Unavailable: runtime-helper operation | Pointer ABI retained, definition not certified | Unknown / Unknown |
| `Decode` | Unavailable through descendants | None, as declared void | Unknown / Unknown |

The three available kernel certificates are current and agree with the inspected
operations. `EndOfData` reads both bytes of `inputPos` and `inputLength` and
defines its byte result on every return. `ReadByte` reads an indirect input byte,
updates both bytes of `inputPos` when needed, and defines `$A0`. Its Unknown
pointee read does not erase the finite write set. `WriteByte` retains finite
carrier/index reads despite an Unknown indirect write, and gains no guaranteed
result merely from that write. Global state reads do not become outgoing
argument obligations. These checks exercise the repaired directional, typed
result, program-state and may-write/definition distinctions independently of TN.

Unknown indexed/pointee footprints are conservative answers. The host's valid
stream and buffer bounds are not compiler certificates. This audit does not
infer a bounded effect from the declared array size or the passed VM vectors.
Conservative flag observations in helper-bearing routines likewise do not
establish that a source-level caller genuinely needs to supply those flags.

## Findings and priorities

### 1. Runtime-helper contracts stop native call composition

`ReadBit` contains `Rsh32` and `Rsh`; `ReadBits` contains `Lsh32` and `Rsh32`.
`ReadHeader` uses `Mul` for record indexing. `BuildTree`, although a source-level
leaf, contains `Mul` and `Lsh` after lowering. The ordinary `MirOp::Call` list
alone therefore understates its calls; this audit separately records helper
sites.

These helper operations already carry structured operands and finite effects.
For example, the 32-bit shifts read `$82–$85` and `$C0–$C3`; their published
may-writes cover `$82–$87` and `$C0–$C7`. The emitted helper bodies inspected here
write within that footprint. Nevertheless,
[acyclic entry composition](../../../src/mir6502/analysis/known_callees/entry_inputs/acyclic.rs)
and [residence admission](../../../src/mir6502/parameter_homes/calls.rs) reject
`RuntimeHelper` operations. Useful finite writes survive in `ReadBit`/`ReadBits`,
but cannot establish a complete supported call boundary.

**Classification:** deliberate coverage limit with broad impact. A future
extension needs helper entry and guaranteed-output contracts as well as existing
may-effects; deleting the rejection alone would not supply those proofs.
This is the first fact-coverage priority because it blocks ordinary generated
code across several kernel routines and their callers.

### 2. Unsupported results erase supported parameter ABI facts

`ReadBits(CARD n)` has an ordinary two-byte argument, but both
`direct_parameter_abi` and `direct_result_abi` are `None`. The
[parameter ABI producer](../../../src/mir6502/lower.rs) checks whether the result
fits its byte/word lane model before recording parameters. A LONGCARD result
therefore suppresses knowledge of `n`, and `ReadHeader`'s calls fail with
`unknown residence ABI`.

**Classification:** avoidable precision loss, not an incorrect call encoding.
Separate parameter knowledge from result support, then represent/prove the
four-byte result contract explicitly. Merely restoring the parameter list does
not prove `$A0–$A3` are defined, and helper restrictions would still remain.
`BuildTree` correctly retains its pointer parameter and result types; pointer
typing itself is not the blocker found here.

### 3. Caller admission depends on conservative operation restrictions

The [source-home guard](../../../src/mir6502/parameter_homes.rs) rejects multiply,
word shifts and variable word arithmetic combined with pointer reads. These
forms occur in `ReadHeader` and `BuildTree`, independently of callee facts.

It also admits address formation for eligible locals/parameters but not ordinary
global objects. Portable `Decode` contains `LeaAddr Global` for `codes` and
`heap`, so it fails caller-body admission. The fixed-address driver expresses
those addresses as constants and reaches per-home call checks instead. There,
both `root` and `current` are blocked while live across `ReadBit`.

**Classification:** placement coverage, with a representation-sensitive gate.
Address formation is not evidence that an existing caller home is exposed.
Any widening still needs the existing storage, expansion and lifetime proofs.
Better entry summaries alone will not overcome these separate body restrictions.

### 4. Simple leaf homes fall between discovery policies

`ReadByte.value` occupies an inline byte even though the routine has no general
temporary ZP allocation. Initial source-home discovery requires a parameter;
the local-placement path requires an ordinary call. This local-only leaf fits
neither path. The portable variant additionally contains global address
formation; the fixed driver does not.

The driver retains `WriteByte.value` inline. Its one-byte parameter is excluded
by the [ZP binding heuristic](../../../src/mir6502/parameter_homes/zero_page.rs),
which expects legacy byte forwarding to suffice. In this emitted variant it
still stores and reloads the inline parameter before staging through `$E2`.
The portable variant eliminates the named parameter home and stages directly
through `$E0`.

**Classification:** discovery/forwarding opportunities, not a fact-soundness
failure. These are small independent controls for subsequent placement work.
No forced-placement experiment was performed, so no saving is claimed.

### 5. Ordinary global writes still lose ZP preservation precision

`ReadByte` has the same logical write footprint in the two layouts. In the
driver, `inputPos` is explicitly at `$0604/$0605`; its summary proves `$E4` is
preserved. In the portable program, `inputPos` is ordinary compiler-owned
storage, selected here at `$2004/$2005`; the same query cannot prove preservation.

The [fixed-backing resolver](../../../src/mir6502/analysis/known_callees/observations.rs)
deliberately does not resolve ordinary storage from the final map. This is the
remaining scope of the earlier fixed-global repair: it prevents an unsafe
assumption but does not exploit known placement or an ownership invariant.

**Classification:** conservative physical-overlap precision. This is a latent
additional restriction, not the sole cause of Huffman's zero named ZP homes.
A repair needs a valid placement/ownership contract at the consuming stage;
borrowing a later map cannot retroactively certify earlier rewrites.

## Size opportunities and limits

| Routine | Portable instructions | Named inline bytes | Direct absolute accesses to those homes |
| --- | ---: | ---: | ---: |
| `ReadHeader` | 1,304 | 8 | 102 |
| `BuildTree` | 906 | 12 | 83 |
| `ReadBits` | 665 | 8 | 45 |
| `Decode` | 221 | 4 | 20 |
| `ReadByte` | 37 | 1 | 2 |

The access column counts emitted unindexed loads/stores/arithmetic against named
home bytes. It indicates where narrower addressing could matter; it excludes
anonymous spills and is not a cost prediction. New reservations can displace
temporary homes or change capture/setup code. `ReadHeader` and `BuildTree`
are the largest source-home opportunities, but need both helper/body coverage
and adequate final placement. Existing whole-image acceptance remains necessary.

The issue-8 fallback also occurs once per audited compile, at
`post-zp-remapping`, but the first failure is **`WriteByte`'s expanded read
footprint**. Logical spill reads become `$E0` in portable storage and `$E0–$E2`
in the driver. This independently confirms the logical/physical transition is
not specific to TN or `Pull`. No pre-#8 Huffman comparison was run, so this audit
does not attribute a byte cost to that fallback.

## Evidence and reproduction

- [facts.json](facts.json): compact selected facts, source homes, temporary ZP,
  helper sites, routine sizes, call admission and placement/contract events.
- [receipt.json](receipt.json): revision, source/image hashes, configurations,
  work counts, execution coverage and limitations.
- [probe.rs](probe.rs) reuses the existing fact inventory and adds retained-input
  operations, selected frames and helper sites. [run.py](run.py) installs and
  removes a temporary test-only include; production source is restored exactly.

Run from the repository root; do not edit/build `known_callees.rs` concurrently:

```sh
python3 docs/benchmarks/6502-huffman-facts/run.py
```

Large raw facts, full MIR, assembly, images and logs remain under
`build/huffman-facts/`. The internal probe completes actual output selection
without invoking the public trace artifact-finishing API. Its trace manifest
therefore says `incomplete`; no detail was truncated. This is distinct from a
failed compile. The selected images and successful audit test establish completion.

Focused execution checks, using the shared low-debug target directory:

```sh
env CARGO_TARGET_DIR=build/checked-arena/shared-target \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  CARGO_PROFILE_DEV_OPT_LEVEL=1 CARGO_PROFILE_TEST_OPT_LEVEL=1 CARGO_BUILD_JOBS=2 \
  cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test huff_dec -- \
  huff_dec_mir6502 huff_dec_vectors_accept
```
