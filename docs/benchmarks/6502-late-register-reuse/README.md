# Late instruction exposure and register reuse

**Qualified:** 1,394 MIR6502 unit tests, 151 integration tests, 63 VM tests and
twelve MADS checks pass. [qualification.json](qualification.json) records the
commands, existing skips and focused rechecks after isolating the older fusion
tests. No shared NIR/frontend contracts changed.

TN shrinks by **six bytes**: `Next` changes from **33 to 30 bytes**, and `Key`
from **37 to 34 bytes**. The independent pointer fixture saves twelve bytes.
These are measured complete images with only `enable_late_register_reuse`
toggled. Both `$3000` and `$41C7` produce the same savings.

| Program | Cartridge before → after | Standalone before → after |
| --- | ---: | ---: |
| TN | 9,853 → 9,847 | 9,960 → 9,954 |
| Independent addition/subtraction | 151 → 145 | 151 → 145 |
| Pointer arithmetic controls | 628 → 616 | 710 → 698 |
| Argument staging | 314 → 314 | 396 → 396 |
| Word load sharing | 301 → 301 | 301 → 301 |

The last two controls retain identical complete outputs, including maps and
relocations. The arithmetic control keeps inlining disabled to expose its
callee bodies; TN and other controls use the ordinary optimized settings.
Unoptimized NIR takes a different staged arithmetic path and remains unchanged
in the integration control. This slice does not optimize every late load.

## Implementation boundary

The [design contract](../../MIR6502_LATE_CLEANUP_DESIGN.md#instruction-exposure-and-register-reuse)
defines a closed expansion of fixed-pair `OffsetPointerByIndirectByte` into
existing typed MIR instructions, including explicit ADC/SBC carry lanes.
Expansion alone emits identical bytes. The shared post-home value analysis
then supports redundant reload removal and LDA-to-TXA/TYA substitution.

The existing terminal transaction checks the deletion intermediate, verifies
the expanded result, preserves callee contracts, replays proofs at the final
placement and requires a smaller packaged image without routine growth. Frames
remain fixed; no new tracker or upstream optimization search was added. A
routine with no accepted reuse retains its original compound form.

The first changes are `5499d302` (contract), `c8951642` (byte-identical expansion),
and `0c8403c3` (gated register reuse). The final qualification enables the flag
in `Mir6502Config::optimized()`; the conservative configuration leaves it off.

## Measurements and checks

[measurements.json](measurements.json) records all twenty comparisons, source
and image hashes, changed routine sizes, work counts and terminal decisions.
TN retains five materializations and 123 emission invocations in each lane.
The arithmetic fixture adds four emission invocations across two terminal
trials; the pointer fixture adds two. Each newly productive terminal trial
uses the existing baseline/candidate emission pair. All materialization counts
remain unchanged. These measurements establish size and work counts, not a
compiler speed improvement.

Focused checks cover expansion identity, initialized carry contracts, live N/Z,
physical aliases, indirect writes, barriers, joins, unchanged frames and the
existing terminal rollback checks. Source-level integration checks cover raw
and optimized NIR, both runtimes, both origins, tracing and LF/CRLF input.
VM controls use independent arithmetic oracles, including carry, borrow,
modulo-65536 results, scratch aliases and exactly-once indirect reads. The
actual TN `Key`/`Next` test also checks equality with the measured images.
Listing checks reconstruct and relocate LF/CRLF assembly with MADS.

The existing plus-one fusion size and VM tests explicitly disable terminal
register reuse to retain their isolated size and internal carrier-read checks.
With both features enabled,
fusion's incremental saving is two bytes because register reuse also removes
three bytes from the unfused baseline. The pointer control and VM tests exercise
the combined pipeline; this adjustment changes no source fixture or oracle.

The measurement probe is
[`late_register_reuse_api.rs`](../../../tools/compare6502/late_register_reuse_api.rs).
It accepts an output directory, writes off/on XEX files and `results.json`, and
asserts non-growth, unchanged materialization counts and complete equality for
unchanged controls. Build it against the selected compiler rlib and its matching
`serde_json` dependency. Local artifacts and logs are under ignored
`build/next-state-audit/`; the compact receipts are committed here.
