# Native 65816 sieve speed comparison

Measured on 2026-09-26 with actionc `73e2a23e` and Calypsi 5.18. This extends the
[CRC comparison](../65816-crc-speed/README.md) with the original `c-bench-64`
standard and bit-packed sieves. All configurations pass the count, complete
flags-array, ABI and protected-memory checks. Debug/release VM measurements
are identical.

## Results

Cycles cover one complete sieve invocation, including array initialization and
all helper calls. Action uses its default optimized pipeline. Both Calypsi
configurations use `-O2 --speed --data-model huge --code-model large`; the second
also uses `--no-cross-call` to disable factoring repeated code into subroutines.

| Workload | Count | Action cycles | Calypsi speed | Calypsi speed, no cross-call | Action / faster Calypsi |
| --- | ---: | ---: | ---: | ---: | ---: |
| Standard, original 8,191 slots | 1,900 | 3,400,699 | 2,865,907 | 2,426,573 | **1.40x** |
| Bit-packed, equal-work 8,191 slots | 1,900 | 9,289,832 | 4,164,094 | 3,839,434 | **2.42x** |
| Bit-packed, original 16,000 slots | 3,432 | 18,623,518 | 8,338,984 | 7,686,402 | **2.42x** |

The original suite repeats the standard kernel ten times and the bit kernel
four times. Those outer repetitions, timer and printing routines are excluded
here. Comparing their original total benchmark times would compare different
amounts of work. The 8,191-slot bit run makes the representation tradeoff clear:
bit packing takes **2.73x** as many cycles in Action and **1.58x** in the faster
Calypsi configuration for the same prime range.

| Kernel | Action code | Action guard regions | Action code excluding those regions | Calypsi speed code | Calypsi no-cross-call code | Flags allocation | Mask table |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Standard | 264 | 27 | 237 | 180 | 181 | 8,191 | 0 |
| Bit-packed | 634 | 135 | 499 | 230 | 269 | 2,001 | 8 |

All sizes are bytes. Code includes every linked helper, excluding unused Action
`Main`. The bit table adds eight initialized bytes for both compilers. The bit
fixture retains its original 2,001-byte allocation even at smaller inputs; the
8,191-slot run initializes only 1,024 bytes, versus 8,191 in the standard kernel.

Disabling Calypsi cross-call factoring costs one byte in the standard kernel
and saves **439,334 cycles (15.33%)**. It removes 31,381 calls to an eight-byte
address-setup helper. For the original bit workload, it costs 39 bytes and saves
**652,582 cycles (7.83%)**, eliminating 46,613 calls to a shared address/mask
helper. The source-level `check_flag` and `clear_flag` calls remain. Both
settings are retained in the evidence; neither compiler was manually inlined
or given a different sieve algorithm.

## Debug guards

Action's standard sieve enters one guard region, costing 25 cycles. The bit
kernel checks at both call sites and callee entries, so the debug checks matter:

| Action workload | Measured cycles | Cycles in guard regions | Remainder after subtraction |
| --- | ---: | ---: | ---: |
| Standard, 8,191 | 3,400,699 | 25 | 3,400,674 |
| Bit-packed, 8,191 | 9,289,832 | 1,159,525 | 8,130,307 |
| Bit-packed, 16,000 | 18,623,518 | 2,330,675 | 16,292,843 |

The original bit workload makes 16,000 check calls and 30,613 clear calls.
Guard-region cycles account for **12.51%** of its Action total. After subtracting
those regions, the arithmetic ratio against faster Calypsi is 2.12x.

These are accounting totals, **not measured guard-disabled builds**. Recognized
entry guard regions also perform stack-frame arithmetic that an actual
guard-disabled implementation must retain or replace. The subtraction must not
be presented as a verified release-mode timing or an exact removable-cycle
forecast. All headline timings use the actual executed bytes.

## Generated-code opportunities

The standard sieve is much closer to Calypsi than the CRC kernels. It already
uses native word arithmetic and CARD-indexed BYTE accesses through Y, but spends
considerable time preparing those accesses and moving loop values.

| Action workload | Stack-addressed instruction cycles | Share | REP/SEP cycles | Share |
| --- | ---: | ---: | ---: | ---: |
| Standard, 8,191 | 1,774,455 | 52.18% | 476,412 | 14.01% |
| Bit-packed, 16,000 | 6,666,076 | 35.79% | 1,866,537 | 10.02% |

Stack-addressed instructions here are opcodes `A3/83/63/E3/C3/23/03/43`.
These are full instruction cycles, not bus-byte counts or forecasts of removable
work. Guard accounting overlaps other categories and must not be added to them.

**1. Direct long-indexed BYTE accesses to known global arrays.** Each standard
array access currently reconstructs the same 24-bit base in direct page:

```asm
SEP #$20
LDA #$00
STA $00
LDA #$E0
STA $01
LDA #$12
STA $02
REP #$20
; load index into Y, select A8, then LDA/STA [$00],Y
```

The three setup sequences in the
[standard Action listing](sieve-actionc-optimized-normal.lst), at
`$01002F..$01003D`, `$010071..$01007F` and `$0100B8..$0100C6`, execute 31,381
times and cost **659,001 cycles (19.38%)**. A known static base plus a CARD index
is a candidate for native absolute-long indexed addressing using X, avoiding
pointer reconstruction. The compiler would need to preserve X requirements,
exact BYTE accesses, relocations and carry across bank boundaries. This is a
target addressing choice and does not require general alias analysis.

**2. Short constant shifts directly in A16.** In both bit helpers, `idx >> 3`
currently goes through direct-page scratch and three memory `LSR` operations.
Calypsi uses three `LSR A` instructions. The current Action sequence after the
captured input is available costs 39 cycles:

```asm
LDA input,S
STA $08
LSR $08
LSR $08
LSR $08
LDA $08
STA result,S
```

Using `LDA input,S; LSR A; LSR A; LSR A; STA result,S` would cost 16 cycles with
the same entry width. The helpers execute 46,613 such shifts in the original bit
workload: **1,072,099 cycles** of potential reduction from this substitution,
before considering surrounding captures. This is a local instruction-cost
estimate, not an implemented speedup. The initialization bound has the same
shift pattern and can benefit too.

**3. Fold adjacent parameter captures into native comparisons.** The standard
sieve repeatedly does `LDA length,S; STA temp,S; LDA index,S; CMP temp,S`.
Comparing directly against the unchanged parameter home avoids the first two
instructions. Those pairs at `$010021/$010023`, `$010062/$010064` and
`$0100AA/$0100AC` cost **332,820 cycles (9.79%)** across this run. Apply only
where storage/effect facts establish that the original parameter value is
still valid and the captured temp has no other required use.

The address setup and comparison-capture totals above identify disjoint
instruction sites. They measure opportunity, not a promise that every cycle
can disappear without replacement instructions. No compiler optimization was
implemented here, and these sieve percentages do not predict Exec-wide savings.

## Inputs and validation

The C suite is at `58036235788d9c9741c67f82e1fb9bb6e2f0e3f6`; both sieve C files
are unmodified in that checkout. The frozen
[C and Action fixtures](../../../tools/native65816-runtime-tests/tests/fixtures/sieve_bench)
retain the original kernel bodies and data definitions while removing benchmark
wrappers, I/O and the unused result global. The Action ports retain CARD loop
variables and BYTE flags; `==&` implements C's compound `&=` operation. The bit
mask complement uses BYTE XOR `$FF`. Division by eight and index modulo eight
are expressed as shifts and masks in Action, with the same unsigned result.

Both compilers use their normal ABI: one CARD/unsigned-int argument (Action
stack, Calypsi A16), returning the count in A16. Code is in bank 1, DP is `$2000`,
entry S is `$5FE0`, and execution starts in native A16/X16. The Action bit table
follows its flags array in data; Calypsi puts the const table after code.

The flags base is compiled at both `$12E000` and `$12FFF9`. The second layout
crosses a bank after seven bytes. Standard inputs are `0,1,2,7,8,9,255,256,257,8191`;
bit inputs add `16000`. Every invocation starts with poisoned flags, then checks
the entire allocation against an independent full-integer prime sieve, including
unchanged unused storage and set padding bits. The original kernel counts prime
2 unconditionally, so its zero-size result is 1.

Calypsi uses widened A16 reads for some non-volatile BYTE values, including the
last bit-mask table element and the final standard flags element. The harness
provides a **read-only one-byte halo** after each object and counts those reads;
it does not treat these ordinary RAM arrays as volatile/MMIO. Writes are limited
to flags and ABI scratch/stack. The table is validated from linked initializer
bytes and protected against writes. Action performs exact BYTE reads and has
zero halo reads. This distinction is retained in `measurements.csv`.

All counts, complete flag states, stack canaries, native width/interrupt state,
DP/DBR restoration and callee-preserved DP bytes pass. Each case runs with I
clear and set, requiring identical records. Twelve compiler/layout artifacts
produce **126 paired records, 252 executions per host build, 504 executions
total**. Only the final fixture/configurations are included in this snapshot.

Timings come from the pinned native VM and its recorded timing correction,
without injected interrupts, wait states or DMA. They run from function entry
through `RTL`, excluding caller argument setup and startup. This is not a
physical-machine wall-clock measurement.

## Reproduction

From the repository root, with Calypsi tools under `/usr/local/bin` and the
recorded `c-bench-64` inputs:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_RELEASE_DEBUG=0 \
  cargo build --release --bin actionc-65816
python3 -B tools/compare65816/sieve.py --suite /Users/michalkurcewicz/atari/c-bench-64
python3 -B tools/compare65816/run_sieve.py target/sieve65816-comparison/manifest.json
python3 -B tools/compare65816/run_sieve.py target/sieve65816-comparison/manifest.json --debug
python3 -B tools/compare65816/report_sieve.py
```

The build verifies extraction against the frozen C inputs, compiles actual LF
and CRLF fixtures, and requires byte-identical images. The runner authenticates
inputs before and after execution. The report requires complete passing results,
identical debug/release records and matching VM qualification evidence. Only
the focused `sieve_bench` runtime target ran; no full backend or Exec
qualification was run. Use `--output` to preserve this historical snapshot when
recording later compiler results.

Evidence: [summary](summary.csv), [all measurements](measurements.csv),
[per-routine entries and exclusive cycles](routines.csv), [instruction profiles](profiles.json),
[provenance](provenance.json). Normal-layout listings: Action
[standard](sieve-actionc-optimized-normal.lst) and
[bit](sieve_bit-actionc-optimized-normal.lst); Calypsi speed
[standard](sieve-calypsi-O2-speed-normal.lst) and
[bit](sieve_bit-calypsi-O2-speed-normal.lst); Calypsi no-cross-call
[standard](sieve-calypsi-O2-speed-no-cross-call-normal.lst) and
[bit](sieve_bit-calypsi-O2-speed-no-cross-call-normal.lst).
Matching `cross-bank` listings are retained alongside them.
