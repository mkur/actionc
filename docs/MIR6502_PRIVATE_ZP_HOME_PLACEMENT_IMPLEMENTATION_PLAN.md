# Implementing private zero-page homes across pointer writes

**Outcome.** A computed store into program data can coexist with a live,
unexposed local in ZP. An explicit physical access, an ABI use or a
callee's scratch can still occupy the proposed byte. Capture the current
TN images and home decisions before the first change. Commit each of the
four slices with its regression checks. [Design note](MIR6502_PRIVATE_ZP_HOME_PLACEMENT.md)

## 1. Express a fixed-size write in a shared assembly review

Extend the argument-relative buffer review with a constant byte extent.
The proposed spelling for `Store(CARD p BYTE value)` is `writes-buffer 1 bytes=1`
alongside `writes-only $A0-$A1`. Together these bound every write to the two
scratch bytes and the one byte addressed by the first argument on entry.
The second argument supplies the value.

Represent the extent as either a constant or the existing count parameter
with its zero convention. Validate the address parameter, extent and matching
fixed footprint in semantic analysis and NIR verification. Carry the typed
fact through SemIR and NIR to classic and MIR6502. Bind the new form to
its reviewed body and to dependent reviews. Preserve the fingerprint encoding
of existing counted-buffer reviews. Document the new spelling. [Shared facts](../src/callee6502.rs) · [Annotation reference](ACTIONC_ANNOTATIONS.md)

**Check.** Both backends match the same fixture. Malformed extents and
incompatible parameters fail validation. A stale review grants no new fact.
General memory queries still report a possible indirect write when the
address is unknown. The home analysis must consume the write declaration
even when the read declaration is absent.

## 2. Carry the destination and scratch through native callers

Reuse the pointee-write summary, argument substitution and numeric observation
paths. A numeric destination occupies its byte range. A forwarded formal
keeps its argument identity and extent until its caller supplies a value.
For a pointer loaded from program data, such as `v(i)`, retain the ordinary
pointee write and the known scratch. This requires no scan of the table's
initializers. Use the retained argument and its verified ABI lanes to
identify that case. A failed lane reconstruction supplies no such evidence.
Apply the same rule to typed native indirect stores. [Native write analysis](../src/mir6502/analysis/known_callees/native_writes.rs) · [Reviewed buffer projection](../src/mir6502/analysis/known_callees/reviewed_buffers.rs)

Compose physical uses on every path through native wrappers. Keep explicit
numeric destinations from either side of a branch. Use matched contracts or
the target ABI for opaque code. Unaccounted physical effects, including those
of a returning callback, still block the affected bytes.

**Check.** Call the same writer through a wrapper with a table entry and with
`$00A2`. Both calls occupy `$A0/$A1`. Only the latter adds `$A2`. Cover a
forwarded argument, a mixed branch and a stale child review. Recheck
`MovePage`'s existing zero-count and pointer-carrier overlap cases.

## 3. Use one byte-availability decision in planning and verification

Establish `ObjectAddressControl` before placement and check logical liveness
at the call. Replace the blanket rejection of an unknown whole-memory
footprint with the existing private-byte query when the invocation's physical
uses are accounted for. Include outgoing arguments, results, reservations
and descendant homes. Check both lanes of a word. Apply the same decision
to the expanded candidate and replay explicit physical observations against
both layouts. Keep entry validation, Action local persistence and the final
image-cost check. [Call checks](../src/mir6502/parameter_homes/calls.rs) · [Selection](../src/mir6502/parameter_homes/cross_call.rs) · [Residence verifier](../src/mir6502/parameter_homes/lifetimes.rs)

**Check.** In a small fixture, keep a caller word live in ZP across a row
write and verify both the word and the destination in the VM. Force one lane
onto `$A1` and require rejection. Exercise a loop back edge and reuse after
last use. Passing `@local` takes that object through the addressable-storage
rules. Passing a pointer value held in a private cell leaves the cell eligible.

## 4. Apply the review to TN and qualify both runtimes

Update `Store` in TN and TNDBG with the same reviewed facts and verify the
refreshed fingerprints in both backends. Keep the assembly payload and use
the general entry at every call. Check the actual
`Copy → Tag(j) → Store` path in cartridge and standalone MIR6502 builds at
`$3000`. Query that call site directly in the test so the earlier `Input`
entry blocker in shipped TN cannot mask the result. [TN](../samples/tn/modern/TN.ACT) · [TNDBG](../samples/tn/modern/TNDBG.ACT) · [TN review tests](../src/mir6502/analysis/known_callees/reviewed_inputs/tn_reviews.rs)

**Acceptance.** The row write permits otherwise available homes for
`Copy.mem`, `Copy.len` and `Copy.files`. Real scratch collisions still fail.
Run the TN VM checks for tag toggling, panel counts and copying. Record the
selected homes, any remaining blocker and packaged byte deltas against the
baseline. A later call such as `Convert` can still prevent final placement.

## Validation commands

Run the required NIR checks for the shared contract slice. Use the affected
home, review and VM targets for the MIR6502 and TN slices. Compare any
failures with the parent under the same settings.

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test

cargo test --lib mir6502::analysis::known_callees::
cargo test --lib mir6502::parameter_homes::
cargo test --test mir6502_cross_call_homes
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test cross_call_homes --test callee_facts --test tn_dispatch
```

## TN qualification

### Store review

The general `Store` review in both samples matches classic and MIR6502.
At the inner `Copy → Tag(j)` call, both lanes of `mem` and `len` and the
byte `files` are live. The reviewed row write permits private homes while
reserving `$A0/$A1` and the other physical writes in `Tag`'s call tree.

The eight shipped load files compared byte-for-byte equal to the pre-change
builds. MIR6502 used origin `$3000`. Sizes below include the XEX headers.

| Source | MIR6502 cart | MIR6502 standalone | Classic optimized cart | Classic optimized standalone |
| --- | ---: | ---: | ---: | ---: |
| TN | 9,594 | 9,733 | 10,642 | 10,866 |
| TNDBG | 12,612 | 12,753 | 13,879 | 14,103 |

At this checkpoint both MIR6502 runtimes selected `$A2` for `Tag.i` and
`$A3` for `Tag.a`. The first entry check on the `Copy → SwapWin` path
reached the unreviewed `Input`. TN kept `Copy.mem`, `Copy.len` and
`Copy.files` at `$33D3`, `$33D5` and `$33D7`.

### Input review

Both backends consume the same opaque `Input` review. The channel arrives in
A, the buffer address in X/Y (low/high), and the byte count in `$A3`.
The review certifies those entry requirements. Memory reads and writes on
return remain unknown because CIO can invoke the installed error handler
and that handler can return to the caller. [TN CIO wrappers](../samples/tn/modern/LIB.ACT)

In both TN sources, `SetWin` branches on its panel argument `n`. The
refresh path assigns `panelDataOld` after `TagAll` and `Inv`, just before
saving the panel. TN can then use `$E2` for `n`, `$E3` for `drvnum` and
`$E4/$E5` for `panelDataOld` on both MIR6502 runtimes. The earlier
`Tag` homes still fit. The first call blocking `Copy.mem`, `Copy.len` and
`Copy.files` is now `Convert(v(j))`, whose writes are unknown to the home
allocator. Their inline addresses are `$33D0`, `$33D2` and `$33D4`.

Each MIR6502 load file is 10 bytes smaller than at the `Store` checkpoint.
Classic optimized load-file sizes are unchanged.

| Source | MIR6502 cart | MIR6502 standalone | Classic optimized cart | Classic optimized standalone |
| --- | ---: | ---: | ---: | ---: |
| TN | 9,584 | 9,723 | 10,642 | 10,866 |
| TNDBG | 12,602 | 12,743 | 13,879 | 14,103 |
