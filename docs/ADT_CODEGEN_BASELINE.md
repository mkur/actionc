# ADT code-quality baseline

Measured 2026-09-09 with compiler `451cab3`, after nested-pattern and guard acceptance. These are complete
small programs, not benchmark-loop extrapolations or a claim of zero-cost ADTs.
The implementation and host oracles are in
[`adt_codegen_audit.rs`](../tools/vm-runtime-tests/tests/adt_codegen_audit.rs).
The checked-in [CSV](ADT_CODEGEN_BASELINE.csv) contains all 24 configurations.

## Method and gate

Three ADT programs are compared with handwritten tagged records having the same
packed payload extent and output behavior:

- `event_dispatch`: a five-byte key/move event, construction, scalar guards and
  dispatch. The manual key path explicitly uses BYTE arithmetic too.
- `guard_snapshot`: a five-byte event whose first guard mutates the original and
  returns false. Both programs keep an independent selector snapshot.
- `nested_result`: a three-byte `Result<Option<BYTE>,BYTE>`, a value-returning
  function, nested patterns and a scalar guard. The manual record explicitly
  validates the outer tag and only the active inline inner tag.

Both forms initialize unused payload bytes to zero. Manual code uses explicit
snapshots and checked tags with Error(100) plus a defensive loop. It is a
reference implementation for these programs, not a source transformation that
would preserve every possible low-level mutation of arbitrary tagged records.

The public compiler builds classic/MIR6502 with cartridge and standalone runtime
at origin $3000. For each of 24 images, the pinned VM executes six input bytes
(`0,7,31,32,127,255`) and two flags: **288 independent whole-memory oracles**.
The complete $0600–$06FF region is checked, including untouched guards, input,
call count and completion marker. The VM revision is `7ec0cc454ebf43b088b7bcd11515533085ea1964`.

Cycles run from object execution entry through the completion-marker store,
including startup. They are CPU cycles, not PAL/NTSC wall time or a claim about
display/DMA timing. XEX bytes include code, data, startup, runtime helpers and
load-file overhead. Both runtime selections happen to have identical cycle
ranges for these programs, but different image sizes.

Correct output, bounded completion, NIR verification before/after optimization,
and preserved memory are hard gates. Performance is an explicit recorded
baseline, not an arbitrary ratio threshold. Invalid values, returning Error
handlers, skipped guards and volatile/fault ordering have separate VM gates in
`variants.rs`, `nested_patterns.rs` and `case_guards.rs`.

## Generated size and cycles

Cartridge-runtime builds; standalone sizes are in the CSV:

| Program | Backend | Manual XEX bytes | ADT XEX bytes | Manual cycles | ADT cycles |
| --- | --- | ---: | ---: | ---: | ---: |
| event_dispatch | classic | 343 | 663 | 370–398 | 1,007–1,062 |
| event_dispatch | MIR6502 | 234 | 587 | 147–148 | 595–645 |
| guard_snapshot | classic | 261 | 557 | 406 | 1,714 |
| guard_snapshot | MIR6502 | 191 | 489 | 170 | 1,070 |
| nested_result | classic | 483 | 821 | 632–680 | 632–1,010 |
| nested_result | MIR6502 | 373 | 604 | 314–345 | 444–658 |

The worst relative cost here is the guard-snapshot MIR program: about **6.3×**
the manual cycles. In this case the guard reconstructs a nullary value as well
as Main constructing its initial event. Matching syntax alone is not the source
of the entire gap.

## Storage, checks and copies

These are **logical NIR** counts, before physical ABI expansion, target temporary
placement or helper emission. Local bytes include generated scalar/address homes;
capture bytes are the subset marked AggregateCapture. They do not measure total
6502 zero-page use, physical callee homes or peak native frame size.

| Program/form | Local bytes raw→opt | Capture bytes raw→opt | Copy sites raw→opt | Static copied-byte sum raw→opt | Direct tag-load compares raw→opt |
| --- | ---: | ---: | ---: | ---: | ---: |
| event_dispatch/manual | 5→5 | 5→5 | 1→1 | 5→5 | 6→5 |
| event_dispatch/ADT | 33→33 | 15→15 | 3→3 | 15→15 | 6→5 |
| guard_snapshot/manual | 5→5 | 5→5 | 1→1 | 5→5 | 4→3 |
| guard_snapshot/ADT | 29→29 | 15→15 | 3→3 | 15→15 | 4→3 |
| nested_result/manual | 16→16 | 9→9 | 2→2 | 6→6 | 10→9 |
| nested_result/ADT | 28→28 | 11→11 | 1→1 | 2→2 | 11→9 |

Copy counts sum static CopyBytes sites, not execution frequencies or stores
inside zero-fill loops. Aggregate ABI expansion introduces additional transfers;
the nested-result ADT having fewer logical CopyBytes does **not** mean fewer
physical copies overall. Tag comparisons are direct uses of byte-tag loads at
the known tag offsets of these audit types. They include validity and dispatch
checks, not just unique validation sites, and are not dynamic check counts.
The CSV also records all comparisons and explicit fault/Error call sites.

## Existing optimization: what worked and what did not

The normal pipeline is used unchanged: verified value/CFG optimization, storage
propagation, scalar promotion, home elision, then verified value/CFG cleanup.
MIR uses its existing selection and rewrite pipeline. No new ADT pass is enabled.

Observed in the saved raw/optimized NIR for `guard_snapshot`:

- Predicate threading removes the repeated `tag=KEY` branch, including the
  false-guard continuation. Its proof uses the already-captured tag, so mutation
  of the original in `Reject()` does not invalidate that SSA value.
- Scalar propagation/promotion removes the pattern-binder stores and forwards
  the surviving payload load directly to output. Unreachable returns disappear.
- Aggregate capture extents and logical CopyBytes sites do not decrease.
  Stored analysis reports `NonScalarStorage`, `UnsupportedType`,
  `AddressTaken` and `AddressRequired` among aggregate promotion blockers.
  Existing scalar home elision is not whole-aggregate copy elision.
- Construction still reaches `zero_aggregate`/`element_loop`: even a five-byte
  object is cleared through a pointer and a SIZE-width loop, then copied into
  the destination. The MIR listing retains word loop-control/address work.
  Nullary reconstruction in the guard repeats that cost.

The next optimization should start with the **shared aggregate initialization
and transfer paths**: reuse existing constant-image/copy/store lowering for small
known extents, and extend capture/copy forwarding only with lifetime, alias,
overlap and effect proofs. General counted-loop/address selection may also help
the remaining clears. Do not add Option/Event-specific rules, erase validation
across effects, or treat all routine-static capture homes as inherently no-alias.
Keep optimization work separate from this correctness/delivery slice.

## Reproduce

From the repository root:

```sh
audit_dir=$(mktemp -d /tmp/actionc-adt-audit.XXXXXX)
ACTIONC_ADT_AUDIT_DIR="$audit_dir" cargo test \
  --manifest-path tools/vm-runtime-tests/Cargo.toml --locked \
  --test adt_codegen_audit -- --nocapture
```

The directory must already exist and be empty. It retains the six complete
sources, raw/optimized NIR, storage facts, 24 listings/images and measurements.
Without that environment variable, the test creates and cleans its own directory.
