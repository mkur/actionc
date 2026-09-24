# Native constant stores, including NULL

Implemented on 2026-09-24 against compiler baseline `91f9cb10`. Nonvolatile
16-, 24- and 32-bit numeric constant stores now use A16 word pairs and an exact
A8 tail for three-byte values. This includes numeric addresses and NULL pointers.
Identical 32-bit halves share one immediate load; a matching 24-bit bank byte
reuses A's low byte after SEP. Reuse stays within the individual store operation.

Every destination byte is written once, in ascending order, without a destination
read or a fourth pointer byte. Volatile stores retain byte instructions, and
symbolic source values retain their byte relocations. Source masking preserves
truncation and zero extension; signed widening still uses explicit casts.
Resolved stack, absolute, DP, symbolic and indexed destination extents are checked
before the constant-store prefix. Address preparation, allocation, ABI, guards
and external memory-access contracts are unchanged. A three-byte private store
starting in A8 retains byte emission when mode switches would make the native
sequence larger. See the [emission contract](../../MIR65816_EMISSION_CONTRACT.md#scalar-instruction-selection).

## Examples

For a 32-bit zero stored through an absolute long address:

```asm
; A16 on entry
LDA #$0000
STA $00CFFF
STA $00D001
```

For a three-byte NULL pointer at `$00EFFF`:

```asm
; A16 on entry
LDA #$0000
STA $00EFFF
SEP #$20
STA $00F001
```

These are independently assembled with ca65 and executed against the previous
bytewise sequences in the qualified VM:

| Absolute zero store | Bytes before → after | VM cycles before → after |
| --- | ---: | ---: |
| 16-bit | 14 → 7 | 17 → 9 |
| 24-bit NULL | 20 → 13 | 24 → 17 |
| 32-bit | 26 → 11 | 31 → 15 |

These examples start in A16. The two- and four-byte native sequences end in A16;
the old sequences and three-byte native sequence end in A8. Whole-image sizes
below include subsequent mode changes. Indirect stores use the same word/tail
strategy through the existing full 24-bit address preparation.

## Exec measurement

The workload is the same frozen optimized Exec `622b139-dirty` input used for
[the preceding Add/Sub slice](../65816-long-arithmetic/README.md): eight task
slots, console/windows, shell and MyDOS. All 120 snapshot input hashes were
verified. The live Exec working tree was not used or modified.

| Measurement | Before | After | Saved |
| --- | ---: | ---: | ---: |
| Compiler-generated routine bytes | 420,058 | 414,261 | **5,797** |
| Same code with measured guards subtracted | 347,806 | 342,009 | **5,797** |
| 147 two-byte constant-store spans | 4,427 | 3,430 | 997 |
| 139 three-byte constant-store spans | 4,020 | 3,056 | 964 |
| 212 four-byte constant-store spans | 7,803 | 4,617 | 3,186 |

Of 498 eligible store spans, 496 shrink and two retain their size. Store spans
include address preparation. Another 650 bytes disappear through neighboring
mode/layout changes. Of 631 routines, 183 shrink and none grows. Routine contracts
apart from address/size, typed MIR operations, initialized data and zero-fill
remain unchanged. All 2,676 compiler guards remain, occupying 72,252 bytes;
compiler-initialized data remains 951 bytes.

The [routine table](changed-routines.csv), [store spans](store-spans.csv) and
[measurement record](measurement.json) retain the detailed accounting and
artifact hashes. This is a compiler-only measurement with guards enabled.
Platform assembly, packager-added data and the XEX were not rebuilt. The initial
public release cap remains 256 KiB of loaded code plus initialized data with
guards disabled; subtracting guard bytes here is an estimate, not an actual
qualified release artifact.

## Validation

- 243 native 65816 library tests passed, with one existing ignored test. The
  new selector cases cover exact instruction bytes, immediate reuse, source
  widths, truncation, fallbacks and extent/transient-stack boundaries.
- 48 integration tests passed across `mir65816_abi`, `mir65816_arithmetic`,
  `mir65816_emission` and `mir65816_o65`.
- Five constant-store VM tests passed in debug and release hosts. They cover
  all scalar widths, both compiler modes
  and initial I states, private locals/parameters, globals, absolute/indexed
  stores, bank crossings, exact bus traffic, canaries, NULL fields around the
  Y displacement limit, volatile instruction widths, ca65 encodings/cycles,
  symbolic source fallbacks and two o65 placements. LF/CRLF sources are compiled
  through the actual parser.
- Another 27 release-host VM consumer tests passed across memory, pointer
  allocation/values, long arithmetic, wide returns, call copies and effects.
- The focused IRQ/NMI probe restores state at every reached instruction in
  both task domains: 178 raw and 104 optimized task/PC/flag combinations,
  including reused A and three-byte tails, plus two seeded schedules. Its
  fixture instrumentation is checked with LF and CRLF input.

Use the qualified wrapper for focused reproduction:

```sh
python3 tools/native65816-runtime-tests/qualify.py --test constant_stores
python3 tools/native65816-runtime-tests/qualify.py --release --test constant_stores
python3 tools/native65816-runtime-tests/qualify.py --release --test preemption native_constant_stores
```

Full hosted Exec qualification remains reserved for the final optimization
commit, as requested. No shared frontend/NIR contract was changed.
