# Implementation plan for shared 6502 callee facts

**Implemented.** The shared 6502 component provides one call contract that classic codegen and MIR6502 can
both query. For handwritten assembly, review the routine once, with LLM
assistance, and check the facts into source. The compiler verifies that the
review matches the selected body. For native Action! routines, each backend
publishes what it can establish about its own generated code. Start with the
direct calls to TN's `Ascii` and `Internal`. [Design and ownership](6502_SHARED_CALLEE_FACTS_DESIGN.md)

The first qualification target was TN's `FormatDirectoryRow`. It should keep its
row pointer in `$AC/$AD` across `JSR Internal` and store the result directly
from A. The baseline MIR home allocator could use an entry review for `Ascii`,
but pointer retention requested exact reads and writes and lost the bound on the indexed table read.
Capture the current listings for `Convert`, `FormatDirectoryRow` and `SetWin`,
plus TN image sizes, in both optimized classic and MIR6502 with ActionCart at
`$3000`. Compare each build to its own baseline. [TN library](../samples/tn/modern/LIB.ACT) · [TN callers](../samples/tn/modern/TN.ACT)

## 1. Share the existing entry fact first

Extract the direct-call review matcher from MIR into a shared 6502 component.
Give both backends a query keyed by the resolved routine and its selected
implementation. Its first use is to check whether the ordinary ABI arguments
satisfy the assembly's entry requirements. The signature already
places `ch` in A. Whether the body physically reads incoming carry is a
different question. Keep the existing `abi-inputs-only <fingerprint>` spelling
and its existing proof scope. Reuse the structural eligibility check for a
straight-line leaf ending in `RTS`. Builds match the checked-in review
deterministically, with no LLM invocation or opcode-based inference of callee
behavior. [Current matcher](../src/mir6502/analysis/known_callees/reviewed_inputs.rs) · [MIR home admission](../src/mir6502/parameter_homes/calls.rs)

**Acceptance.** Both backends answer the same way for the current `Ascii` and
for a changed body with its old fingerprint. A caller that fails to supply the
declared ABI lanes still fails the home-admission check. Moving the matcher
alone should preserve the current MIR output.

## 2. Carry the rest of the review through the frontend

Extend that contract with independent fields for complete may-read and
may-write ranges over the whole invocation, normal-return preservation, and
the result's location on return. Give unknown and explicitly empty footprints
distinct representations. Retain the current spellings for preservation,
partial clobbers and result equality. A partial clobber list cannot establish
a complete write footprint. [Current annotation definitions](ACTIONC_ANNOTATIONS.md)

Use a versioned fingerprint for the whole review. The proposed source spelling
below illustrates `Ascii`. The range endpoints are inclusive. `none` expresses
an empty footprint in either direction. The existing entry-only fingerprint
continues to certify just that one field.

```text
;@actionc reviewed-6502 <fingerprint>
;@actionc abi-inputs-only
;@actionc reads-only asciiMap[0..3]
;@actionc writes-only $A0
;@actionc preserves $AC/$AD $AE/$AF
;@actionc returns A=$A0
```

Parse the declarations once. Resolve the table in SemIR and carry its storage
ID, byte offset and extent through NIR routine metadata. Give classic the
same resolved declaration through its semantic input. Check valid IDs and
bounds in the verifier and print the metadata for inspection. On the target
side, bind the review to the ABI, assembled bytes and typed relocations,
including the identity, extent and backing of each referenced object. Make
this check available before either backend optimizes a caller, including
forward calls. Recheck if a placement trial changes a binding used by the
proof. [Semantic IR](../src/semantic/ir.rs) · [NIR lowering](../src/nir/lowerer.rs) · [Classic semantic input](../src/codegen/semir.rs)

**Acceptance.** A table relocation that preserves its identity and assumptions
keeps the review valid. Changing the code, ABI, table binding, backing or
extent invalidates it in both modes. Malformed declarations produce a source
diagnostic. A missing or stale review leaves only independent ABI guarantees.
Test a missing read clause separately from `reads-only none`.

## 3. Switch the producers and callers

- **Handwritten assembly.** Enter the reviewed facts for both leaves, using
  `internalMap[0..3]` for `Internal`. Route both backends' handwritten-callee
  queries through the matcher. Audit existing uses of classic annotations and
  MIR opcode-derived summaries before switching them over. Migrate reviewed
  cases that those optimizations rely on. An unmatched body must take the
  conservative path, including when an old opcode analysis could infer more.
- **Native Action! routines.** Adapt the existing MIR typed-operation and control-flow
  summaries and classic's generated routine effects to the same queries. Each
  producer may leave a field unknown. Tie physical scratch and register facts
  to the implementation that established them. Exercise a native routine
  through both backends. [MIR callee summaries](../src/mir6502/analysis/known_callees.rs) · [Classic routine facts](../src/codegen/model.rs)
- **Classic callers.** Replace the annotation path that initializes unknown
  effects to `known_empty()`. Apply preservation only to the named locations
  on normal return. Use the result equality when A is still live. Test a call
  with preservation alone so an unlisted write stays possible. [Classic annotation handling](../src/codegen.rs)
- **MIR callers.** Let ZP home admission query ABI sufficiency and actual
  overlap separately. Let pointer retention carry a preserved pair across the
  call when it still denotes the same destination. Delete a spill store only
  when no possible callee read overlaps it and no other use needs it. Elide
  the `$A0` reload only while A still holds the result and any required flags
  are accounted for. [Pointer retention analysis](../src/mir6502/analysis/record_bases/retention.rs) · [Materialization](../src/mir6502/materialize/pointer_retention.rs)

**Acceptance.** Compare every published field for identical assembly in both
modes. In a small caller with a live destination pointer, verify that the pair
reload, redundant spill store and return-slot reload disappear under the full
contract. Withhold the read footprint, then supply one that
overlaps the spill byte. In each case the observable store must survive even
though the pair is preserved. Indirect calls and unsupported assembly bodies
must still act as barriers for these transformations.

## 4. Qualify the TN calls and the shared IR boundary

Extend the TN VM test to exercise both assembly leaves in both backends. Run
all 256 input bytes with both incoming carry values. Compare A and `$A0` to
the expected conversion, seed and check both preserved pairs, and check the
return stack pointer. Trace program-data reads and writes against the declared
table and return slot, accounting separately for instruction fetches and
ordinary call-stack traffic. Run the directory-row cases through the compiled
caller. [TN VM test](../tools/vm-runtime-tests/tests/tn_dispatch.rs)

Inspect the before/after listings for the three baseline callers. Record the
removed saves, restores and reloads at the affected call sites, the unchanged
17-byte leaf bodies, and the per-routine and total image size changes. Whole
image size alone cannot establish which optimization fired.

Run the focused backend and VM targets below. For changes to SemIR, NIR, its
printer or verifier, also run the three required shared-contract checks.
Compare failures with the pre-change checkout and identify any new regression.

```sh
cargo test --test mir6502_call_abi --test mir6502_cross_call_homes --test mir6502_bounded_call_pointer_retention
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml --test tn_dispatch --test cross_call_homes --test bounded_call_pointer_retention
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```


## Qualification

The copy-loop extension adds argument-relative buffer footprints and a separate
physical-entry claim. The same checked-in review covers the byte-identical
reverse-copy loops in TN and the runtime fixtures. A zero byte count covers
256 bytes. MIR can retain a caller pointer across the copy when it establishes
the actual spans and their disjointness. The counted-copy fixture saves 10
bytes and 14 cycles per proven call. The unknown destination retains the
original code. [Review syntax](ACTIONC_ANNOTATIONS.md) · [Copy-loop fixture](../fixtures/runtime/bounded_call_pointer_retention.act)

The following sizes are bytes in the TN ActionCart build at `$3000`. Routine
sizes are listing ranges. The image row is the packaged XEX size. Both sides
used the same working-tree Action! versions of `Position` and `Relpos`. For
classic, the comparison moved TN's source origin and both library location
counters from `$2C00` to `$3000` in the measurement copies.

| TN artifact | MIR before | MIR after | Classic before | Classic after |
| --- | ---: | ---: | ---: | ---: |
| Ascii | 17 | 17 | 17 | 17 |
| Internal | 17 | 17 | 17 | 17 |
| Convert | 126 | 126 | 173 | 173 |
| FormatDirectoryRow | 333 | 288 | 344 | 344 |
| SetWin | 636 | 612 | 804 | 804 |
| XEX | 9,902 | 9,837 | 10,650 | 10,659 |

At the first `Internal` call in MIR's `FormatDirectoryRow`, the caller saves
18 instruction bytes. It drops two absolute spill stores, two absolute loads
and two zero-page stores to restore the pointer, and the load from `$A0`.
MIR emits the following instructions at that site.

```asm
        JSR.A proc_Internal
        LDY #$01
        STA ($AC),Y
```

The loop over columns 2 through 9 and the conversion in `SetWin` also store
directly from A. The two later conversions with different source and
destination offsets still restore a saved pointer. `Convert.s` remains in
`$E2/$E3`. Classic already used A for the directory-row store. Five classic routines gained one
two-byte load each. `Tag` gained a two-byte return-slot load and lost a
three-byte local load. Together these changes account for the nine-byte
increase. [TN call sites](../samples/tn/modern/TN.ACT)

The caller regression checks an omitted read clause, a read of the actual
spill address and a stale body. The shared matcher tests compare every
published field across the two modes and exercise a generated Action! callee.
The TN VM exercises both leaves in both modes over 256 inputs and both
incoming carry values. It checks fetches, stack accesses, data accesses,
return values and preserved pairs, and runs the directory-row oracle through
the complete application. [Caller regression](../tests/mir6502_cross_call_homes.rs) · [TN VM](../tools/vm-runtime-tests/tests/tn_dispatch.rs)


The NIR sweep verified and optimized all 52 fixtures. The `enum_values`
snapshot still differs from its expected text. The full Rust test command
stops at five existing library failures in constructor storage, scalar
capture and parameter handoff. Focused integration and VM runs also reproduce
existing assertions in cross-call homes, parameter handoff, register-copy
forwarding and directional callee facts. The same failures were checked
against the pre-change compiler. The counted-copy MADS relocation check
passed with LF and CRLF listings.
