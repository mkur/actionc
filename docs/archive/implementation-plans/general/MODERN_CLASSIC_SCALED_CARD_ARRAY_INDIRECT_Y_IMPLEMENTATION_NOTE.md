# Modern/Classic Scaled CARD-Array `(zp),Y` Implementation Note

Status: implemented and validated

Date: 2026-07-18

Scope: `--profile modern --backend classic`

Implementation progress:

- Slice 1 is implemented: `ScaledIndirectY` is an accepted index-address and
  pointer-dereference proof mode for two-byte elements with byte-ranged
  indexes.
- Slice 2 is implemented for the six existing effective-address consumers:
  the base high byte is corrected with the live `ASL` carry and stored once.
  This reduced the TN code range from 10,654 to 10,642 bytes.
- Slices 3 and 4 are implemented for direct and computed byte-index word
  consumers. Word call arguments, word-plus-constant arguments, ABI spills,
  scalar loads, constant stores, and array-pointer-value stores now share the
  carry-preserving scaled-address preparation.
- Slice 5 is implemented for profitable two-address assignments by staging the
  complete source word before preparing the destination. Twenty-five of the 28
  remaining TN full-address sites migrated; three zero-net-gain transfers keep
  the established lowering.
- The final TN modern/classic code segment is 10,546 bytes and the load file is
  10,558 bytes, both 96 bytes smaller than the pre-Slice-3 baseline.
- The apparent garbled-screen regression was traced to Atari800 inheriting an
  Atari 400/800 machine configuration while the runner only supplied an XL/XE
  ROM path. It was not caused by the emitted instruction change. The runner
  now selects XL/XE explicitly and isolates saved cartridge state for
  `--no-cart` runs.
- The runtime boundary fixture verifies indexes 0, 1, 127, 128, and 255 across
  fixed, descriptor-backed, typed-pointer, and signed-word storage. It covers
  loads, constant and scalar stores, direct-call and computed-index consumers,
  an overlapping two-address copy, array-pointer values, a call that clobbers
  `Y` and `$AE/$AF`, a destination that overwrites its own descriptor, and
  corrected-high-byte wrap from `$FF` to `$00`.

## Objective

Lower byte-indexed two-byte array elements without first materializing the full
16-bit element address. Keep the low byte of `2 * index` in `Y`, add the carry
from `ASL` to the base pointer high byte, and let 6502 `(zp),Y` addressing add
the low-byte offset.

The optimization applies to `CARD ARRAY`, `INT ARRAY`, and equivalent
two-byte pointer indexing when all of the following are proved:

- the element width is exactly two bytes;
- the index value is in `0..=255`;
- evaluating the index once at this point is legal;
- the selected consumer can keep the scaled `Y` value live until the indirect
  load or store;
- any required source staging is correct and profitable.

This is a proof-guided lowering choice, not a post-emission byte peephole. The
legacy profile and the MIR6502 backend are out of scope.

## Current TN Baseline

Generate the baseline with:

```sh
cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-source-listing \
  samples/tn/modern/TN.ACT > target/tn-modern-classic-before.lst
```

The original roadmap estimate was 33 full-address sequences and therefore a
nominal opportunity of about 132 static bytes at four bytes per site. Before
Slices 3 through 5, a fresh listing at `eafa82a` contained 28 adjacent
`ASL`/`PHP` full-address sequences:

| Routine | Remaining full-address sites |
| --- | ---: |
| `Sort` | 4 |
| `Handle` | 3 |
| `SetWin` | 3 |
| `Xloop` | 3 |
| `Copy` | 2 |
| `Draw` | 2 |
| `Delete`, `DrawWinFrame`, `IsDirectory`, `IsProtected`, `IsTagged`, `MakeJmp`, `Path`, `Rename`, `Tag`, `TagAll`, `View` | 1 each |
| **Total** | **28** |

There were also six sites already using the intended `ASL`/`TAY` addressing
model: two in `Sort`, two in `Copy`, one in `SetWin`, and one in `Handle`.
Their base materialization now loads the high byte without storing it first,
then corrects it with the live `ASL` carry and stores it once.

After this implementation:

- 25 of those 28 sequences use scaled `(zp),Y` lowering;
- the listing has 31 scaled `ASL`/`TAY` sites: the six prior consumers plus 25
  migrated consumers;
- the 25 migrations have a 100-byte gross expectation at four bytes each;
- consumer staging costs four bytes in aggregate, yielding a measured 96-byte
  code-segment and load-file reduction;
- the complete scaled-address work, including Slice 2, reduces the load file
  from 10,666 bytes after trampoline elision to 10,558 bytes, a total of 108
  bytes.

The 28 remaining sites split into two main consumer families:

- 19 word-valued call arguments, including `v(i)`, `v(j - 1)`,
  `v(copytab(k))`, and the `v(i) + 1` cases in `IsProtected` and
  `IsDirectory`;
- 9 word assignments, including array-to-array moves in `Sort`, array-pointer
  stores in `SetWin`, and indirect/array transfers in `Handle`.

The three retained full-address sites are all two-home transfers where keeping
one scaled offset live requires four bytes of staging or register restoration,
exactly cancelling the four-byte address saving:

| Routine/source | Reason retained |
| --- | --- |
| `Sort`: `v(j+gap)=h` | Computed scaled destination plus scalar source needs alias-safe source staging; net zero bytes. |
| `Handle`: `dirsectors(nestLevel)=currentDir^` | Scaled destination plus direct pointer dereference needs a second Y home or full staging; net zero bytes. |
| `Handle`: `currentDir^=dirsectors(nestLevel)` | Direct pointer destination plus scaled source needs Y restoration or staging; net zero bytes. |

Keeping these established forms is the explicit profitability decision; none
is an unclassified missed consumer.

## Target 6502 Sequence

For a byte index already in `A`, a descriptor-backed array with base bytes
`base`/`base+1`, and a word consumer that wants the high byte first, replace:

```text
ASL A                 ; A = low(2*i), C = high(2*i)
PHP
CLC
ADC base
STA ptr
LDA #0
ROL A                 ; carry from base-low addition
PLP
ADC base+1
STA ptr+1
LDY #1
LDA (ptr),Y           ; high byte
DEY
LDA (ptr),Y           ; low byte
```

with:

```text
ASL A                 ; A = low(2*i), C = high(2*i)
TAY
LDA base              ; LDA/STA preserve ASL carry
STA ptr
LDA base+1
ADC #0                ; ptr += high(2*i) * $100
STA ptr+1
INY
LDA (ptr),Y           ; high byte
DEY
LDA (ptr),Y           ; low byte
```

The same prepared address may start at the low byte by omitting the first
`INY`, then use `INY` for the high byte.

For the address setup plus initial high-byte selection, the descriptor-backed
form removes four static bytes and nine nominal cycles. Indexed reads can pay a
one-cycle page-cross penalty per `LDA (zp),Y`, so the realized read saving is
normally seven to nine cycles. Indexed stores do not have the conditional read
penalty.

### Correctness identity

For `i` in `0..=255`, define:

```text
y = (2*i) mod 256
c = floor((2*i) / 256)       ; exactly the carry produced by ASL
ptr = base + 256*c
```

The 6502 effective address is then:

```text
ptr + y = base + 2*i         (mod 65536)
```

Because `y` is even and at most 254, `INY` selects the high byte without
wrapping `Y`. The zero-page pointer pair must not start at `$FF`; existing
Action scratch pairs satisfy this constraint.

## Existing Infrastructure To Reuse

The modern classic generator already contains most of the required model:

- `IndexAddressProof` classifies byte-ranged indexes and two-byte elements as
  accepted `ScaledIndirectY` addresses in `src/codegen/proof.rs`;
- `byte_index_effective_address` constructs a direct-index effective address
  in `src/codegen/array.rs`;
- `emit_effective_address_pointer_and_y` already emits the `ASL`/`TAY` model
  for selected load, store, and arithmetic consumers;
- `emit_index_low_expr_to_acc` can evaluate read-only computed byte indexes
  such as `j + gap`, `j - 1`, and nested byte-array indexes;
- processor state already tracks actual `Y` changes and invalidates carry on
  `ASL`/`ADC` conservatively;
- `EffectiveAddressLowered` provides optimization-log observability.

The implementation should generalize these paths. It must not recognize
individual TN routines or rewrite emitted `ASL/PHP/.../PLP` byte strings.

## Implementation Plan

### Slice 1: Make the proof and address contract explicit

1. Use the explicit accepted `IndexAddressMode::ScaledIndirectY` mode for
   exactly two-byte elements with a byte-ranged, read-only index. Keep
   unsupported widths and side-effecting indexes rejected.
2. Keep the proof independent of the final index home. A direct one-byte slot
   is the first materialization case, but a proved computed byte expression
   must be representable by a later slice without weakening the proof.
3. Define the prepared-address contract explicitly:
   - the pointer pair contains `base + 256 * ASL_carry`, not the final element
     address;
   - `Y` contains `(2 * index) & $FF`, optionally plus byte offset 0 or 1;
   - the consumer owns subsequent `INY`/`DEY` changes;
   - calls, labels, machine blocks, and unknown clobbers end the contract.
4. Update `docs/CODEGEN_PROOFS.md` and focused proof tests for accepted and
   rejected scaled modes.

Do not extend generic `StorageSlot::IndirectIndexedY` to silently mean a
dynamic nonzero `Y` base. That slot currently describes byte offsets from a
fully materialized pointer, and changing its meaning would make unrelated
load/store helpers unsafe.

### Slice 2: Emit the ideal carry-preserving base setup

Status: implemented. The temporary rollback was reversed after the reported
garbled screen was isolated to an Atari800 400/800-versus-XL/XE configuration
mismatch.

1. Split base loading from generic pointer materialization in
   `src/codegen/array.rs`:
   - load/store the base low byte to the selected scratch pointer;
   - load the base high byte without storing it first;
   - consume the still-live `ASL` carry with `ADC #0`;
   - store the corrected high byte once.
2. Support the same base categories already accepted by the effective-address
   proof:
   - inline array bases via immediate low/high bytes;
   - pointer and descriptor array bases;
   - typed pointer variables with a two-byte pointee.
3. Route the existing `emit_effective_address_pointer_and_y` two-byte arm
   through the new helper. This removes the redundant high-byte store at the
   six TN sites that are already scaled.
4. Use tracked generator emitters for all instructions so A/Y/flags, zero-page
   values, memory invalidation, and optimization logs stay accurate.
5. Add a debug assertion for a valid non-wrapping zero-page pointer pair.

The instructions between `ASL` and `ADC #0` must be restricted to operations
that preserve carry. In the intended sequence these are `TAY`, `LDA`, and
`STA`. Do not call a generic base materializer whose future implementation
could insert `CLC`, arithmetic, a call, or a machine block.

### Slice 3: Migrate direct-index word consumers

Status: implemented.

Add consumer-level helpers that prepare the scaled address once and then emit
the required byte order. Prefer these helpers before the generic
fully-materialized-lvalue fallback.

1. In `src/codegen/call.rs`, update all word argument transfer shapes:
   - direct word to `A`/`X`;
   - word plus a third register argument;
   - high byte to `X` with low byte staged or stacked;
   - word arguments spilled at ABI offsets three and above.
2. Load high then low as:

   ```text
   prepare scaled Y
   INY
   LDA (ptr),Y
   TAX or stage high
   DEY
   LDA (ptr),Y
   ```

   Preserve existing left-to-right argument evaluation and final Action ABI
   register placement.
3. In `src/codegen/arith.rs`, add a scaled effective-address source to the
   word-plus-constant argument path. This covers `Value(v(i) + 1)` without
   materializing `v(i)` first.
4. In `src/codegen/assign.rs`, extend `EffectiveAddressStoreSource` with an
   array/pointer-value source so assignments such as `v(i) = s` and
   `dirnames(i) = pathBuf` can use the scaled target. Load or stage source bytes
   according to the existing alias/evaluation-order rules.
5. Keep existing scalar-load, scalar-store, and arithmetic effective-address
   consumers on the same shared preparation helper.

Expected TN coverage from this slice includes the simple-index sites in
`Path`, `Draw`, `DrawWinFrame`, `IsTagged`, `IsProtected`, `IsDirectory`,
`Tag`, `SetWin`, `Xloop`, `Delete`, `View`, `Rename`, `Copy`, and `Handle`.

### Slice 4: Materialize computed byte indexes once

Status: implemented.

1. Add a preparation entry point that accepts a proved byte index expression,
   evaluates it once into `A`, and immediately performs `ASL`/`TAY`.
2. Reuse `emit_index_low_expr_to_acc` for supported read-only expressions.
   Do not duplicate expression semantics in the address emitter.
3. Accept examples such as:
   - `j + gap`;
   - `j - 1`, `i - 1`, and `c - 1`;
   - `copytab(k)` where the inner byte-array lookup is read-only.
4. Reject or fall back when the expression contains a routine call, an unknown
   raw/machine operation, a volatile read, a pointer write, or an unproved
   word-range result.
5. Ensure index evaluation finishes before base-pointer setup and that it does
   not occur a second time in the consumer.

This slice targets the remaining computed-index sites in `Sort`, `Draw`,
`TagAll`, `SetWin`, `Copy`, and `MakeJmp`.

### Slice 5: Handle two-address word assignments profitably

Status: implemented for profitable scaled-source/scaled-destination transfers;
the three zero-net TN forms retain their existing lowering.

The array-to-array and array-to-indirect assignments in `Sort` and `Handle`
need two addresses while only one scaled value can live in `Y`.

1. Add a narrow word staging plan rather than changing generic lvalue meaning:
   - evaluate and load the source word exactly once;
   - stage it in a non-overlapping word temporary, or in registers plus one
     byte temporary when that is cheaper;
   - prepare the scaled destination address;
   - store both bytes through `(zp),Y`.
2. Preserve source evaluation before destination stores and conservatively
   assume an indirect destination may alias absolute or pointer-backed source
   storage unless the entire source value is staged first.
3. Use existing zero-page temp allocation and callee-effect rules. No temporary
   may cross a call in this slice.
4. Compare emitted size with the existing full-address form. Keep the old
   lowering when staging consumes the expected saving or increases code size.
5. Log each profitable application and a clear proof-attempt rejection for
   unprofitable or unsafe cases.

The goal is not an unconditional zero count. Every surviving TN full-address
sequence must have a documented safety or profitability reason.

### Slice 6: Observability and documentation

Status: implemented for accepted lowerings and profitability results. Rejected
index proofs distinguish side-effecting/volatile, non-byte, unsupported-width,
and unsupported-base cases.

1. Record `EffectiveAddressLowered` with a message that distinguishes scaled
   `(zp),Y` lowering and with the actual local byte delta, not the historical
   four-byte estimate when staging changes the result.
2. Extend proof-attempt output to distinguish:
   - accepted scaled indirect addressing;
   - non-byte index;
   - side-effecting/volatile index;
   - unsupported element width or base;
   - consumer clobbers `Y`;
   - source alias/staging failure;
   - unprofitable two-address lowering.
3. Update `docs/CODEGEN_PROFILES.md` and link this note from
   `MODERN_OPTIMIZATION_ROADMAP.md` when implementation begins.

## Safety Requirements

- `ASL` carry must reach `ADC #0` without any intervening carry-changing
  instruction.
- Processor state must describe the new actual carry; it must not pretend that
  `(zp),Y` page crossing updates CPU carry.
- No consumer may depend on the flags left by the old full-address addition.
  Consumers that need arithmetic or compare flags must establish them after
  address preparation.
- `Y` provenance for `2 * index` must not be confused with the unscaled index
  provenance used by ordinary byte arrays.
- The index is evaluated exactly once and in source order.
- The base descriptor/pointer is read before an indirect store can alias and
  overwrite it.
- Both source bytes are staged before an indirect destination store when
  runtime aliasing could otherwise change the second source byte.
- Calls and machine blocks remain barriers unless existing structured effects
  explicitly prove preservation.
- The implementation retains the backend's existing binary-arithmetic
  assumption for `ADC`; it does not introduce a new decimal-mode contract.
- Compatible-profile output must remain byte-for-byte unchanged.

## Test Plan

### Focused codegen tests

Add modern/classic tests for each supported base and consumer:

- descriptor-backed `CARD ARRAY` load to a scalar;
- inline local `CARD ARRAY` load and store;
- `CARD POINTER`/`INT POINTER` indexed load;
- scalar, constant, and array-pointer-value stores;
- direct word argument in `A`/`X` and with a third byte in `Y`;
- ABI-spilled word argument;
- word-plus-one call argument;
- computed indexes `j + gap`, `j - 1`, and nested `copytab(k)`;
- profitable array-to-array assignment;
- an aliasing two-address assignment that must stage or fall back.

Positive byte-shape assertions should require `ASL A`, `TAY`, base loads,
`ADC #0`, and `(zp),Y`, and reject the full-address
`ASL A`/`PHP`/`CLC`/`ADC`/`PLP` shape in the optimized region.

### Boundary tests

Exercise indexes `0`, `1`, `127`, `128`, and `255`, with bases chosen so that:

- adding `Y` does not cross a page;
- adding `Y` crosses a page;
- adding the ASL high carry wraps the pointer high byte;
- the high element byte is at offset 255 and `INY` still does not wrap.

Add negative tests for:

- an unbounded two-byte index;
- a three-or-more-byte element;
- a side-effecting call index;
- volatile or unknown index evaluation;
- a consumer that needs `Y` before the memory access;
- a call or machine block between preparation and use;
- overlapping scratch/source storage;
- an unprofitable two-address case.

Where practical, add a small runtime fixture that fills a word table with
distinct values at indexes 0, 127, 128, and 255, then verifies loads, stores,
and call arguments. This catches carry and page-cross errors that listing-only
tests cannot.

### Regression checks

Run:

```sh
cargo fmt --check
cargo test
cargo test --test compatibility tn_stability_check -- --ignored
```

Then generate and inspect TN artifacts:

```sh
cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-load \
  samples/tn/modern/TN.ACT > target/tn-modern-classic-after.xex

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-source-listing \
  samples/tn/modern/TN.ACT > target/tn-modern-classic-after.lst

cargo run --quiet --bin actionc-emit -- \
  --profile modern --backend classic --emit-map \
  samples/tn/modern/TN.ACT > target/tn-modern-classic-after.map
```

If `../action-compiler-vm` and the ROMs are available, run the focused runtime
fixture and a TN startup smoke test using `docs/ACTION_COMPILER_VM_USAGE.md`.

## Acceptance Criteria

- Modern/classic emits the carry-correct scaled `(zp),Y` sequence for every
  proved and profitable two-byte/byte-index consumer.
- Indexes 128 through 255 address the correct page; index 255 accesses both
  bytes without `Y` wrap.
- The 28 current TN full-address sites are either removed or individually
  explained by a logged safety/profitability rejection.
- The four-site `Sort`, three-site `SetWin`, three-site `Xloop`, and two-site
  `Copy` hotspot groups show the expected reductions without special cases.
- The six already-scaled TN sites no longer store the uncorrected pointer high
  byte before `ADC #0`.
- The optimization log's summed local savings reconcile with decoded listing
  deltas; the final report also states the net load/code-segment change.
- Legacy-profile/classic output is byte-for-byte unchanged for focused fixtures
  and the TN stability check.
- MIR6502 output and NIR contracts are unchanged.
- All focused tests and `cargo test` pass; runtime boundary checks pass when the
  VM is available.

## Suggested Commit Slices

1. `classic: model scaled indirect-y card addresses`
2. `classic: use scaled indirect-y for word call arguments`
3. `classic: use scaled indirect-y for computed indexes`
4. `classic: stage profitable two-address word assignments`
5. `docs: record scaled indirect-y TN results`
