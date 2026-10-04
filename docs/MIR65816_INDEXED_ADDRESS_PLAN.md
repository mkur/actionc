# MIR65816 bounded indexed address calculation

## Objective and baseline

Implement priority 4 from the frozen Exec compiler-output audit: form indexed
three-byte addresses without the generic bytewise pointer/index scratch loop
when the complete offset is bounded. This selects existing typed MIR65816
`AddressOf` operations; it does not change source semantics or NIR.

Baseline compiler: `8ce1bd08` (compiler sources at `5dbd8c02`). Frozen Exec:
`154bcf5a45605aeee57de5420ec4604e4ff7b30c`, with 166 loaded source files and 899
routines. Code sizes are 326,894 bytes optimized unchecked, 439,350 optimized
guarded, and 374,735 raw unchecked. Initialized compiler data is 1,385 bytes.
Platform assembly and hosted Exec boot qualification are outside these totals.

All 26 audited sites remain in the current output, occupying 2,572 bytes:

| Slice | Sites | Current bytes | Replacement model | Modeled saving |
| --- | ---: | ---: | ---: | ---: |
| Constant numeric indices | 10 | 756 | 120 | 636 |
| Bounded unsigned BYTE indices | 16 | 1,816 | 527 | 1,289 |
| Total | 26 | 2,572 | 647 | 1,925 |

These are static template estimates, not measured implementation savings.
Mode selection, adjacent consumers and branch relaxation may change the result.
Keep the frozen source and per-profile layout hashes for every comparison.

## Contract and ownership

MIR65816 owns the instruction/address strategy. Consume stable typed address,
index, width and home facts already present at this boundary. Do not consult
SemIR, inspect source strings or add NIR transformations. Retain allocation,
frame sizes, temporary homes, ABI padding and stack bounds.

Form an address from a complete private three-byte base without dereferencing
it. Resolve read-only pointer bindings at the actual operation site; destination
writes must use the allocated writable home. Check source/index/destination
widths, complete stack reach and overlap before emitting any instructions or
changing tracked state. Retain existing symbolic-address, component-consumer
and DP/X selector ownership. Unsupported geometry keeps the existing fallback;
malformed homes retain explicit diagnostics.

Fold unsigned numeric constants with host u64 arithmetic. Require a valid
nonzero stride and `index * stride + displacement <= 65535`; do not truncate an
out-of-range offset into eligibility. Reuse checked unindexed pointer copying
or constant addition. Keep the established identity/overlap restrictions.

For dynamic indices, initially admit only captured unsigned BYTE values with
complete private stack homes and `255 * stride + displacement <= 65535`.
Read exactly one index byte, explicitly clear hidden B, and compute the bounded
scale/displacement in A16. Preserve carry from adding the low pointer word into
an exact A8 bank-byte add. The resulting pointer wraps modulo 24 bits. Use
existing domain scratch only where needed for non-power-of-two strides; do not
allocate more scratch or extend X residency. Require disjoint complete base and
result homes for this slice. No external memory is read during address formation.

Keep conservative barriers, typed effects and selected-code replay authoritative.
No value or flag proof may outlive the operation. Do not reorder captures or
broaden pointer forwarding windows across stores, calls or other operations.
Signed/wider dynamic indices, unbounded scales, unsupported bases and incomplete
homes retain their existing paths. The general implementation must not contain
special cases for Exec routines.

## Implementation commits

1. Commit this plan before compiler changes.
2. Add bounded constant-index `AddressOf` folding through existing checked native
   pointer copy/add selection. Cover zero/nonzero indices, strides, displacement,
   bank carry/wrap, borrowing and refusal boundaries. Measure frozen Exec and
   commit this slice with its focused tests and contract update.
3. Add unsigned BYTE-index selection with full preflight, exact-byte zero
   extension, bounded power-of-two and profitable non-power-of-two scaling,
   and full bank carry. Check interaction with symbolic and component-address
   consumers. Measure and commit separately.
4. Qualify the final backend, rebuild all three frozen profiles, record actual
   candidate coverage/savings and unchanged ABI/allocation/data facts, and commit
   final results. Record any safety or profitability refusals explicitly.

## Validation and completion

Use focused MIR65816 unit and native runtime targets during each slice. Include
independent ca65 encodings and VM arithmetic, exact private reads/writes and
canaries, unchanged external traffic, both entry accumulator widths and poisoned
hidden B, index extrema, stride/displacement bounds, 16-bit bank carry and 24-bit
wrap. Check unsupported/malformed homes atomically, partial overlap, existing
borrowed sources, consumer ownership and reference/replayed output. Cover raw
and optimized builds, guards on/off, two relocated o65 placements, and IRQ/NMI
restoration at instruction boundaries including after earlier nested calls.
Exercise LF and CRLF through any changed newline-sensitive fixture preparation.

After the final compiler change run the affected backend library and root
integration targets and the full native release suite using
`python3 -B tools/native65816-runtime-tests/qualify.py --release --no-fail-fast`.
Keep compiler, runtime, disassembler and test inputs stable throughout each
qualification invocation. Do not exclude baseline failures; existing opt-in
external comparisons may remain ignored. Run shared-contract checks only if
the implementation actually changes shared frontend/NIR contracts.

Use nonincremental builds with reduced debug information and monitor disk space.
Preserve unrelated working-tree changes. Keep bulky images/inventories under
`target/`; commit compact measurements/provenance under `docs/benchmarks/` and
update the emission contract and this plan. Validate all 166 source hashes,
per-profile layouts, routine signatures/arguments/results, frames, homes, stack
bounds and initialized/zero-fill data against baseline. Separate measured code
savings from the model and from any future hosted Exec qualification.

## Constant-index slice

Implemented constant folding through the existing checked pointer selector.
Frozen optimized unchecked Exec is 326,122 code bytes, saving 772 bytes against
the baseline. All 166 source hashes, layout bytes, 899 routine contracts/homes
and data agree with baseline. Measurements are recorded in
[results.json](benchmarks/65816-indexed-addresses/results.json).

Focused validation: 17 indexed-address library tests; 3 new native tests plus
the existing 8 address-selection and 6 pointer-value tests. New tests cover
ca65 encodings, register state, exact private traffic, boundary/fallback values,
LF/CRLF preparation, guards, raw/optimized modes, relocation and IRQ/NMI after
nested calls. Full backend qualification follows the BYTE-index slice.

## BYTE-index slice

Implemented exact-width unsigned BYTE capture and bounded A16 shift/add, followed
by low-word and bank-byte addition. Existing INDEX scratch handles non-power-of-two
strides; checked pointer-source resolution handles borrowed parameter/local bases.
Four selector unit tests cover encodings, bounds, types, stack reach and atomic
refusal. Seven native tests cover all 256 indices, ca65/register/traffic oracles,
borrowed locals after calls, fallbacks, relocation, line endings and IRQ/NMI,
including non-power-of-two scaling after earlier calls. Existing address/pointer
runtime targets remain green.

Optimized unchecked Exec is now 324,615 bytes: 1,507 bytes below the constant slice
and 2,279 below baseline. All 26 audited sites are selected: constant spans shrink
from 756 to 98 bytes and BYTE spans from 1,816 to 501 bytes. Their actual 1,973-byte
span saving exceeds the original 1,925-byte model. Four additional indexed-address
sites save 274 bytes; neighboring mode/branch changes save a net 32 bytes. There
are no refused audited candidates. Wider/signed indices, unbounded offsets and
unsupported homes retain existing fallbacks. Full backend qualification remains
the final step.
