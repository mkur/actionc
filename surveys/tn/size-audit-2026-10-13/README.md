# TN code-size audit, 2026-10-13

TN's next substantial size work should focus on retaining record addresses and
reducing intermediate value transfers. The current MIR6502 image contains
15,030 bytes of code and 1,575 bytes of loaded data. Disabling small-leaf
inlining or small-loop unrolling saves no bytes in TN. The remaining size is
spread across real application work and conservative code generation; this
audit does not identify one switch or abstraction whose removal would recover
kilobytes.

These measurements cover the maintained TN and TNDBG sources, the cartridge
runtime, origin `$2C00`, and compiler commit
`2e8aaa06efefa24d24e8881450bd858970c5640e`. They reuse the final qualified
images from the [bitmap and captured-bound work](../bitmap-scans/README.md).
The qualified compiler SHA-256 is
`f31bf597713027b4f8bcd97bbc486218eab381d77dc728e8cf18483708ef1cbf`.
[measurements.json](measurements.json) pins every image, normalized listing,
source file and analysis script. No production compiler or TN source changes
were made for this audit.

| Image | Code bytes | Loaded data bytes | XEX file bytes |
| --- | ---: | ---: | ---: |
| TN, classic optimized | 16,460 | 1,604 | 18,076 |
| TN, MIR6502 optimized | 15,030 | 1,575 | 16,617 |
| TNDBG, classic optimized | 19,387 | 2,164 | 21,563 |
| TNDBG, MIR6502 optimized | 18,200 | 2,142 | 20,354 |

Each file also contains two loaded `RUNAD` bytes and ten XEX header bytes.
Every annotated listing byte reconciles with its XEX address and value.
MIR6502 is already 1,459 bytes smaller than classic for TN, and 1,209 bytes
smaller for TNDBG. A local MIR/classic regression is therefore a useful quality
signal within an overall improvement.

The code/data split needs source interpretation. Six popup menu blocks are
declared as raw procedures, documented as data in
[TN.ACT](../../../samples/tn/modern/TN.ACT) and
[TNDBG.ACT](../../../samples/tn/modern/TNDBG.ACT), and used through `@name`.
They occupy 204 bytes in every image. Their strings and relocated addresses
partly decode as instructions, including apparent branches and memory accesses.
The analyzer checks their address-only uses and classifies their entire payload
as data. It retains the original printed-form counts separately in JSON.
Ordinary assembly routines remain code; their bytes are not automatically
compiler overhead. This corrects accounting without changing the images.

The largest TN routines account for 4,589 code bytes, 30.5% of its code:

| Routine | MIR6502 bytes | Classic bytes | MIR minus classic |
| --- | ---: | ---: | ---: |
| Copy | 859 | 844 | +15 |
| Handle | 576 | 598 | −22 |
| ChangeTagged | 479 | 538 | −59 |
| TagsExtent | 479 | 413 | +66 |
| MyDosRead | 435 | 531 | −96 |
| SetWin | 392 | 380 | +12 |
| MyDosName | 376 | 509 | −133 |
| MyDosParse | 360 | 465 | −105 |
| Window | 319 | 338 | −19 |
| SortBatch | 314 | 472 | −158 |

The directory, MyDOS, location and panel-directory modules contribute 6,867
code bytes, 45.7% of TN's code. They contain substantial provider, selection,
cache and presentation behavior, alongside repeated record operations.
`TagsExtent` is the clearest large routine where MIR6502 still loses size to
classic. Conversely, MyDOS parsing and naming, and batch sorting, are already
considerably smaller under MIR6502.

TNDBG adds 3,737 file bytes over TN: 3,170 code bytes and 567 data bytes.
The included modules have identical code sizes between the two roots.
Debug `Copy` grows to 1,541 bytes and `Dbg` occupies 1,452 bytes. Additional
strings explain 424 of the 567 added data bytes. Debug instrumentation should
therefore be tracked separately from production TN when assessing growth.
The complete routine inventory and matched backend comparisons are in
[routines.csv](routines.csv) and [backend-deltas.csv](backend-deltas.csv).

The following TN MIR6502 instruction categories are disjoint and sum to
15,030 bytes. They describe physical encodings, including necessary work;
they are not estimates of removable overhead.

| Instruction category | Bytes | Share of code |
| --- | ---: | ---: |
| Direct parameter/local home operands | 4,071 | 27.1% |
| Direct named-spill operands | 465 | 3.1% |
| Fixed pointer and ABI areas | 1,784 | 11.9% |
| Calls and routine tail jumps | 1,455 | 9.7% |
| Other branches/jumps and returns | 1,402 | 9.3% |
| Direct global/static operands | 1,365 | 9.1% |
| Indirect/indexed memory instructions | 799 | 5.3% |
| Remaining register, immediate and memory instructions | 3,689 | 24.5% |

The 4,071 home-operand bytes comprise 1,926 parameter and 2,145 local bytes.
Those loaded homes occupy only 263 and 234 data bytes, respectively; named
spills occupy 48. Loaded strings occupy 499 bytes, globals 288, menus 204,
and inline argument descriptors 39. Reducing instruction traffic is a larger
opportunity than reclaiming the named spill storage alone. Anonymous zero-page
accesses can belong to source or runtime memory as well as compiler scratch;
the audit does not label them all as spills. Indexed accesses are classified by
addressing form rather than assuming an indexed base identifies every element.

Boundary measurements overlap the categories above. TN spends 434 bytes on
recognized parameter-entry capture and 78 bytes on thirteen cartridge `SArgs`
entry sequences, including their descriptors. Pure load/transfer suffixes
immediately before calls or routine tail jumps occupy 1,666 bytes. That suffix
count is a reproducible partial view of call preparation, not a complete ABI
cost or a removable-byte estimate. Canonical input bytes zero, one and two use
A, X and Y; later bytes use `$A3` onward. `$A0/$A1` are result locations.
There is no assumed mandatory caller mirror of A/X/Y in `$A0..$A2`.

Record address construction is the strongest repeated pattern. The analyzer
finds 132 full two-lane setups occupying 1,310 bytes. A typical setup is:

```asm
LDA param_BatchBegin_batch       ; 3 bytes
STA $AC                         ; 2 bytes
LDA param_BatchBegin_batch+1     ; 3 bytes
STA $AD                         ; 2 bytes
```

Twenty-eight setups, occupying 280 bytes, repeat the same source and pointer
pair within a syntactically straight-line region. Twenty-seven follow an
indirect store. `TagsInit` accounts for eight, `TagsReset` three, and
`BatchBegin` two of these detected repeats. The matcher intentionally misses
some shorter setup variants and repetitions across branches; 280 bytes is a
candidate-pattern budget, not an upper bound on the broader opportunity or a
qualified saving.

`BatchBegin` is 86 MIR bytes versus 73 classic bytes. After its first setup,
it rebuilds the same base for each subsequent field store. Reuse needs proof
that the field write cannot alter the cell holding the captured pointer, and
that the pointer pair, registers and flags remain valid. Calls, machine blocks,
volatile accesses and unknown aliases need conservative barriers. The single
TN repeat without a detected indirect-store barrier is in `ActivateLocation`:
the compiler reads `directory` into a local, rebuilds the base, then reads
`unit`. Even this case needs storage and machine-state proofs. Actual sites and
the detected barriers are recorded in [motifs.csv](motifs.csv).

Intermediate carriers are the second strong direction. `TagsExtent` loads
the captured `currentExtent` word into both a named spill and its source local:
four absolute stores occupy twelve bytes before comparison. The comparison
then constructs a numeric Boolean and stores it in another use of the spill.
The source snapshots protect later behavior; deleting them or rereading the
record would change the proof obligation. The compiler should instead prove
when one value carrier can serve the consumers. This requires complete lane
uses, dominance, home observability and alias/effect information through joins,
narrowing/widening, stores and calls.

Several smaller patterns are measurable. Seven accumulator Boolean diamonds
occupy 49 bytes, and six stored Boolean diamonds occupy 76. Numeric results and
their homes can be semantically required, so these 125 bytes are not all
avoidable. Eighteen duplicate literal BYTE exits occupy 90 bytes: replacing
each earlier five-byte exit with a three-byte jump gives a raw 36-byte saving
for this detected subset, before final qualification. Nine word-store
sequences use four register/stack shuttle bytes each, another 36-byte pattern
budget. Their full input capture and low/high store order must be preserved.
These are useful bounded slices, but none explains TN's size by itself.

An isolated compiler snapshot tested the speed/size policy hypothesis. Only
the optimized configuration's small-leaf inlining and small-loop unrolling flags
were made controllable; the working compiler checkout was unchanged. The
snapshot's default output matched both qualified XEX files exactly.

| MIR6502 policy control | TN file bytes | TNDBG file bytes |
| --- | ---: | ---: |
| Current optimized policy | 16,617 | 20,354 |
| Small-leaf inlining disabled | 16,617 | 20,325 |
| Small-loop unrolling disabled | 16,617 | 20,354 |
| Both disabled | 16,617 | 20,325 |

TNDBG's `PrintHexB` shrinks from 100 to 70 code bytes without small-leaf
inlining; one additional loaded data byte makes the whole-image saving 29.
It remains larger than classic's 44 bytes. Its widened values, global `h`
updates and result carriers therefore deserve attention beyond policy alone.
The original 22-byte MIR `HexDigit` body remains emitted even after its direct
calls are expanded. The current inliner permits bounded growth for cycle
savings and retains original bodies, as described in the
[inliner plan](../../../docs/MIR6502_SMALL_LEAF_INLINER_PLAN.md).
These controls are static size experiments, not new VM or timing qualification.
They do not support blanket disabling of either feature. Executable, patched
configuration and image hashes are in [policy-controls.json](policy-controls.json).

The recommended order of work is:

1. Strengthen captured-address identity and effects sufficiently to retain
   record bases across proven field sequences. Start with straight-line
   routines such as `BatchBegin`, `TagsReset` and `TagsInit`; measure final
   routine and whole-image bytes, home storage, occupied zero page and cycles.
   Keep unknown pointer writes and machine effects as barriers. Extend scope
   only when the proofs support it.
2. Reduce intermediate carriers with lane-aware use/definition and home
   liveness proofs. Use `TagsExtent` as the production size sentinel and
   `PrintHexB` as the debug value-flow sentinel. Preserve source captures,
   public ABI behavior and observable memory access order. The existing
   [rewrite workflow](../../../docs/MIR6502_REWRITE_WORKFLOW_PLAN.md) provides
   the transaction and final-budget framework.
3. Follow with small return-sharing, Boolean-consumer and word-store slices,
   each with a measured size benefit and explicit machine-state proof. Treat
   inliner size policy as a secondary option, given the measured 0/29-byte
   result. The source's capacity, publication and selection-readiness guards
   enforce different contracts; this audit supplies no basis for removing
   them as duplicate validation.

No new runtime claims are made. Existing production behavior qualification is
documented with the captured-bound results. This audit checked the real parser
with LF/CRLF listings and sources, byte-exact accounting for all four qualified
images, actual LF/CRLF output equality for all eight policy controls, and exact
default-control identity with both qualified MIR images. Compiler suites were
not repeated because production compiler, IR and sample behavior were unchanged.

Reproduce the census from the retained qualification artifacts:

```sh
python3 surveys/tn/audit-size.py --check
python3 surveys/tn/audit-size.py \
  --baseline target/tn-arithmetic/motion/baseline \
  --out-dir target/tn-size-audit/recheck
```

The optional policy probe rebuilds an isolated snapshot and all controls; it
requires additional disk and build time. Its configuration comparison should
contain exactly the two environment-controlled flags. To repeat it:

```sh
python3 surveys/tn/probe-size-policies.py \
  --revision 2e8aaa06 \
  --baseline target/tn-arithmetic/motion/baseline \
  --out-dir target/tn-size-audit/policies-recheck --crlf
```

Local full listings, XEX files and the isolated snapshot are retained under
`target/tn-arithmetic/motion/baseline` and `target/tn-size-audit/policies`.
Generated CSVs and JSON in this directory preserve the reviewable measurements
without adding binary artifacts to the repository.
