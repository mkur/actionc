# TN combined window frame experiment

2026-11-13. Compiler source revision `0f777d68`.

**Reject both prototypes for TN's code-size priority.** Both reduce checked
cursor operations and improve execution time, but increase the complete
MIR6502 image by 363–364 bytes. The maintained TN/TNDBG and compiler sources
remain unchanged. The prototypes and bulk artifacts are generated under
ignored `build/tn-window-frames/`.

## Compared implementations

The baseline opens a window with `SavePos`, image `Alloc`, pointer `Push` and
another `SavePos`. Closing performs `RestorePos`, `Pull`, `Free` and another
`RestorePos`. These perform eight cursor validations and updates per pair.

Both candidates reserve the entire image plus ten metadata bytes with one
checked `Alloc`. Closing captures and validates the cursor once, checks the
frame bounds before metadata access, restores the screen and parent geometry,
and rewinds once. Public `Window`/`CloseWindow` signatures stay equal.

| Variant | Frame layout | Extra frame state |
| --- | --- | --- |
| Baseline | Parent geometry (4), image (N), image pointer (2), window geometry (4) | Existing stack cursor |
| Linked | Previous-frame pointer (2), parent geometry (4), window geometry (4), image (N) | Global current-frame pointer |
| Trailer | Same byte layout as baseline, reserved/released as one unit | Existing stack cursor |

`Push`/`Pull`, `SavePos`/`RestorePos` and the generic allocator remain needed
by other callers. Permanent screen/line buffers retain their existing 1,000
bytes inside the 2 KB arena. This experiment changes only the window pair,
plus initialization of the linked version's current-frame pointer.

## Complete image sizes

| MIR6502 program/runtime | Baseline | Linked | Trailer |
| --- | ---: | ---: | ---: |
| TN cartridge | 10,290 | 10,654 (+364) | 10,653 (+363) |
| TN standalone | 10,415 | 10,779 (+364) | 10,778 (+363) |
| TNDBG cartridge | 13,373 | 13,737 (+364) | 13,736 (+363) |
| TNDBG standalone | 13,498 | 13,862 (+364) | 13,861 (+363) |

Classic optimized also grows: linked adds 362 bytes to TN and 365 to TNDBG;
trailer adds 368 and 372 respectively, in either runtime. The full
[scorecard](results.json) includes every image hash and changed routine.

For the TN cartridge MIR6502 build:

| Routine instruction bytes | Baseline | Linked | Trailer |
| --- | ---: | ---: | ---: |
| Window | 319 | 456 | 483 |
| CloseWindow | 47 | 258 | 234 |
| Init | 100 | 106 | 100 |

Grouping operations removes calls to shared code without removing that code:
the other users still retain all the generic primitives. The new window
bodies contain their own metadata addressing, frame-bound checks and word
arithmetic. Window plus CloseWindow grow by 348/351 instruction bytes. Linked
also adds six Init bytes; remaining complete-image growth is storage/layout
overhead. This is why fewer runtime operations do not yield a smaller image.

## Runtime and memory results

Representative TN cartridge MIR6502 cycles, including each routine's RTS and
excluding the common 15-cycle call wrapper:

| Window case | Baseline pair | Linked pair | Trailer pair |
| --- | ---: | ---: | ---: |
| 11×5 saved rectangle | 6,755 | 5,896 | 5,901 |
| Three nested windows, total | 34,217 | 31,650 | 31,665 |
| 39×22 debug rectangle | 46,570 | 45,693 | 45,698 |
| 40×24 full screen | 51,358 | 50,481 | 50,486 |

The linked small-window pair saves 859 cycles (12.7%); the full-screen pair
saves 877 (1.7%). Image copying and drawing dominate larger cases. Both
candidates reduce `ArenaCursor` entries from eight to two per pair. These
timings describe the recorded images and layouts, rather than a universal
cycle delta at every placement.

All variants retain ten metadata bytes per window and identical arena
high-water marks. Maintained UI scenarios still peak at `$2583` for TN and
`$274C` for TNDBG. The full-screen stress case peaks at `$27B2`; an exact-fit
case reaches `$2800` and closes back to its original mark.

Failure behavior differs intentionally in the isolated experiment. With one
byte too little space, every version enters `ArenaStop` with exhaustion code
2 and preserves the upper arena guard. Baseline has already advanced its
cursor to `$27FD`; both candidates leave it at `$27C0`, with arena bytes,
geometry and window count unchanged. They reserve the frame before applying
window state. This is not a claim of byte-identical failure states or a
drop-in production replacement. Geometry tests use valid screen rectangles;
hostile/corrupted frame metadata is outside the qualification claim.

Raw and optimized NIR retain the existing `[8192,10240]` normal-return ranges
for `ArenaCursor` and `Alloc` in all three source versions. Capacity and
invalid-cursor guards remain enabled; no opaque assembler allocator replaces
the checked source.

## Reproduction and validation

```sh
python3 tools/compare6502/tn/window_frames.py --check --ui --mads
```

The tool verifies the current release VM dependency build, compiles its small
probe against those existing libraries, generates isolated source copies and
compiles 24 full-program images: three implementations, TN/TNDBG, classic
optimized/MIR6502 and cartridge/standalone. It performs no compiler edits.
It reuses the maintained arena and UI oracles with source-directory and
measured-image bindings in generated copies of their harnesses.

Qualification completed:

- Five successful cases per image: small, three-level nesting, debug, full
  screen and exact fit; 336 window open/close calls with independent expected
  saved pixels, restored full screen, geometry, window count, arena cursor,
  permanent-buffer bytes, guards, CPU stack and primitive-entry counts.
- Twenty-four one-byte-short failure cases, including atomic failure checks
  for both candidates.
- All three versions pass the maintained checked-arena oracle: 1,120
  success/fault calls each (3,360 total), including actual LF/CRLF source
  parsing, both backends/runtimes, nesting and original UI high-water marks.
- All three pass the maintained cartridge dispatch, menu, panel, directory
  and copy oracles in both build modes. The VM-loaded images match the
  measured images before execution.
- Forty-eight byte-identical MADS roundtrips of all images with LF and CRLF
  listings. The generator's final check reproduces the scorecard.

Use `--reuse-images` only with an existing completed run, identical generated
source hashes and an unchanged committed compiler. It skips image compilation
while rerunning the focused execution measurements; `--ui` and `--mads` still
perform their requested checks. Raw per-call data, generated source, listings,
images and qualification logs stay in `build/tn-window-frames/`.
