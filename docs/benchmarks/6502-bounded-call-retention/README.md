# Bounded call pointer retention results

The optimized MIR6502 backend can retain a pointer setup across an inspectable
machine call when the actual destination footprint is confined to owned RAM
and leaves the pointer homes unchanged. The neutral copy fixture saves ten
emitted bytes, fourteen VM cycles and two pointer-setup stores per proven call.
Unknown destinations retain the previous code.

TN's `Strcat` remains 113 bytes. Its incoming destination pointer is unknown,
and the callers' mutable globals and unchecked allocator do not establish a
bounded arena or a closed-entry contract. The proof does not assume those
facts. Its four fresh TN/TNDBG cartridge/standalone images remain byte-identical
to the preceding qualified images, including the required post-copy reloads.

Implemented through the [three-slice plan](../../MIR6502_BOUNDED_CALL_POINTER_RETENTION_PLAN.md).
The facts and consumer are committed as `dbeac306` and `60d456f8`; the final
qualification enables the optimized default and rejects incoming-flag readers.
[baseline.json](baseline.json) freezes the neutral listing controls at
`c7cf44c9`; [results.json](results.json) records image/source hashes, storage
checks, sample cycles and qualification counts. Compiler changes are confined
to MIR6502 analysis and selection.

## Measurements

| Input | Before | After | Saving |
| --- | ---: | ---: | ---: |
| Neutral emitted bytes | 180 | 170 | 10 |
| Neutral XEX bytes | 192 | 182 | 10 |
| `ReadAcrossCopy` body bytes | 56 | 46 | 10 |
| Sample call VM cycles | 156 | 142 | 14 |
| Executed pointer-setup stores | 4 | 2 | 2 |
| `UnknownCopy` body bytes | 69 | 69 | 0 |
| TN cartridge XEX bytes | 9,857 | 9,857 | 0 |
| TN standalone XEX bytes | 9,982 | 9,982 | 0 |
| TNDBG cartridge XEX bytes | 12,952 | 12,952 | 0 |
| TNDBG standalone XEX bytes | 13,077 | 13,077 | 0 |

All eight neutral raw/optimized-NIR, runtime and origin pairs agree. Origins
are `$3000` and `$41C7`. The cycle sample reads the destination before and
after copying two source bytes; it includes the helper and shared test wrapper.
The copy helper's zero count retains its existing 256-byte behavior.

Routine frames, storage capacities, zero-page placement and reservation
lengths remain unchanged. Deferred buffer addresses move with the shorter
code; both source and destination still have complete 512-byte allocations.
Four pre-implementation listings assemble to the exact feature-off XEXs.

## Proof and fallback

The existing final physical map resolves actual register and scratch bytes
at the call. A bounded worklist analysis of the leaf machine body tracks
known bytes, independently known flags and definite flag definitions. It
collects every reachable direct write and indirect-Y destination. Differing
values at joins become unknown; unknown Y covers all 256 byte indices.
Footprints are recomputed from these inputs, never cached by callee ID alone.

Every indirect destination byte must lie in owned ordinary RAM. This excludes
zero page, stack, hardware, code and wrapping/unknown addresses. Source reads
must remain in the supported RAM range. Direct scratch writes also belong to
the footprint. The resident-pointer analysis invalidates a captured value when
the copy can change its source home, even if its scratch pair is preserved.

The initial ISA subset covers loads/stores, register transfers, index
increments/decrements, immediate comparisons, conditional branches,
CLC/SEC/CLV, NOP and one terminal RTS. It rejects a branch unless its input
flag is definitely defined on every incoming path. Calls, stack instructions,
arithmetic, indexed-X addressing, unresolved machine operands and unsupported
instructions retain the previous barrier. Nonzero relocation addends,
high-byte relocations and incomplete byte-address projections are also
rejected. Limits are 512 machine bytes and
4,096 worklist steps. This is an effect proof over possible executions, rather
than a claim that arbitrary loops terminate.

Existing source-equality, live-state and structural transaction checks own
setup deletion. Proposed callees and final placement reconstruct the same
proof before acceptance. Strict final-image savings, unchanged frames and
published callee facts remain required. Disabling parent call retention also
disables the new consumer, even when CFG retention stays enabled. Pointer-home
promotion retains its previous policy.

`enable_bounded_call_pointer_retention` is enabled by `Mir6502Config::optimized()`
and disabled by the conservative default. No allocator, entry, NIR or source
aliasing contract changes. Extending the proof to TN needs verified argument
provenance and allocator bounds; a census of visible calls is insufficient.

## Validation and reproduction

Eight new focused unit cases exercise loop footprints, zero/256 counts,
unknown inputs, ownership/extent boundaries, malformed branches, unsupported
instructions, incoming flags, relocation projections, changed callees and missing placement. Selection
tests also check explicit parameter-home writes, unknown sources, live-state
preservation, reporting, repeated application and fallback. Existing MIR6502
unit, integration, emission/load and affected VM targets pass.

The new VM target executes 2,952 calls across raw/optimized NIR, both runtimes,
both origins, feature-off/on and actual LF/CRLF source files. It covers every
byte count, destination/source/pointer-home aliases and a copy that changes a
pointer home before its next dereference. Independent memory models compare
buffers, parameter writes, ordered indirect reads, returned state, guards,
stack balance and setup traffic. Eight LF/CRLF MADS assemblies also reproduce
the selected default at both origins.

Fresh TN/TNDBG images are bound to the previous exact image hashes. Existing
string, parameter-home overwrite, Tag, dispatch and panel VM checks therefore
execute the same qualified bytes. Exact totals appear in the scorecard.

```sh
CARGO_INCREMENTAL=0 cargo test --locked --lib bounded_machine_copy
CARGO_INCREMENTAL=0 ACTIONC_BOUNDED_CALL_IMAGES="$PWD/build/bounded-call/measured" \
  cargo test --locked --test mir6502_bounded_call_pointer_retention
CARGO_INCREMENTAL=0 ACTIONC_BOUNDED_CALL_VM_RESULTS="$PWD/build/bounded-call/vm-results.tsv" \
  cargo test --locked --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test bounded_call_pointer_retention -- --nocapture
CARGO_INCREMENTAL=0 cargo test --locked --test mir6502_bounded_call_pointer_retention \
  -- --ignored --nocapture
python3 tools/compare6502/tn/bounded_call_retention.py --check
```

MADS must be on PATH or selected by `ACTIONC_MADS`. The ignored TN check needs
the [preceding qualified images](../6502-argument-x-bitwise/README.md#qualification-and-reproduction).
Full local qualification logs, frozen listings and generated images remain
under ignored `build/bounded-call`. The generator checks those logs and frozen
controls as well as the measured outputs; rebuilding the neutral fixture alone
does not recreate the pre-implementation baseline.
