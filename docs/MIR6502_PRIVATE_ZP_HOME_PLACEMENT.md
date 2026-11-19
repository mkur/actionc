# Private zero-page homes in MIR6502

**Zero page is the default memory home for ordinary locals and parameters
whose address the compiler controls.** In optimized MIR6502, enter each such
object into ZP allocation. Choose inline storage for that object when the
pool has no suitable bytes for its lifetime. A value carried entirely in
registers needs no memory home.

This is the target policy. The current selector can reject every ZP byte for
a live object when a call's write summary is unknown. The implemented
`Store` review and the measured TN placements provide the baseline.
[Current selector](../src/mir6502/parameter_homes/cross_call.rs) · [TN qualification](MIR6502_PRIVATE_ZP_HOME_PLACEMENT_IMPLEMENTATION_PLAN.md#tn-qualification)

## Allocating a home

1. Establish address control from verified IR before placement. Taking
   `@mem`, fixing the address, or exposing an alias to the cell uses the
   existing addressable-storage rules. Passing or dereferencing a pointer
   held in `mem` keeps the cell private. [Address-control analysis](../src/mir6502/analysis/storage_ownership/source_objects.rs)
2. Start with the pool for the selected target and runtime. Honor fixed
   program storage and permanent runtime reservations. For each candidate,
   exclude bytes needed by other live values, instruction scratch, call
   arguments, results and callee homes during its lifetime. A word needs two
   adjacent bytes. A contiguous ABI capture frame must fit as a unit.
   [Pool selection](../src/mir6502/parameter_homes/zero_page.rs)
3. Account for loop back edges and every value the home must hold. A local
   read on a later invocation needs its value preserved in between, including
   its initial contents where applicable. Reuse a byte after its last live
   use. A conflict rules out that placement. Try the remaining slots before
   falling back to inline for the affected object or capture frame. Continue
   allocating other objects in the same routine.
   [Lifetime and persistence rules](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#zp-home-lifetimes-and-persistent-storage)

## Calls and pointer writes

An ordinary program-data access has the same aliasing rules for private ZP
and inline homes. The allocator can place a local across an indexed array
store or a copy between ordinary records even when the runtime destination
address is unknown. General memory analysis still records the pointee effect
and preserves access order. Explicit physical accesses occupy their actual
range. Preserve the argument identity and extent of a forwarded destination
so that a caller supplying a numeric address reserves the corresponding bytes.
Account for writes that can overwrite a copy routine's own pointer and redirect
subsequent writes.

For a native call, use typed MIR to compose the pool bytes used by argument
setup, the callee and its descendants. A live value needs a home that
survives the call. A value that dies before the call can release its byte
after its last argument use. Entry requirements and result delivery still apply.
Discovery and verification after expansion use the same byte conflicts and
check explicit observations against the final layout. [Call checks](../src/mir6502/parameter_homes/calls.rs) · [Residence verification](../src/mir6502/parameter_homes/lifetimes.rs)

For opaque assembly, runtime services and callbacks, the target ABI or a
matched review supplies the pool preservation guarantee. If a returning
external call has no such guarantee, its unprotected pool bytes are
unavailable to values live across it. An ABI wrapper can protect specific
bytes by saving and restoring them when no intervening code can read or
update the displaced value, including through reentry. Entry requirements describe
the incoming arguments. Live values need a return guarantee as well.
Values on either side of the call can use those bytes during disjoint
lifetimes. [Shared callee contracts](6502_SHARED_CALLEE_FACTS_DESIGN.md)

## TN examples

- `Copy.mem`, `len` and `files` survive `Tag(j)`. Its reviewed
  `Store(v(i), value)` writes a directory row and uses `$A0/$A1` as scratch.
  Reserve the scratch and other callee homes. The row's numeric address is
  unnecessary for placing the three private locals. Calling the same writer
  with `$00A2` also occupies `$A2`. Passing `@mem` exposes `mem` itself.
  [Tag](../samples/tn/modern/TN.ACT#L355)
- `Copy.j` holds the source drive across the first `SwapWin()`. On that
  path, `SetWin(n, 0)` copies two nine-byte panel records. Those ordinary
  copies can coexist with a private caller home. `SetWin`'s physical homes
  and the copy helper's ABI and scratch still occupy pool bytes. Later,
  `j` holds the file index across `Bget` and is read by `copytab(files) = j`.
  A single ZP home must also survive that returning CIO boundary.
  [Panel switch](../samples/tn/modern/TN.ACT#L515) · [Copy loop](../samples/tn/modern/TN.ACT#L915)

## Cost and qualification

Pool availability admits a home without a separate trial against its inline
version. When eligible objects compete for the same bytes, prefer placements that
save more at executed uses, including repeated accesses in loops. Count
argument capture, spills, reloads and any call protection in that comparison.
Use the existing whole-image cost check on the resulting layout. Qualify the
policy with final home maps, packaged image bytes and VM cycles on the same
workload for each runtime. For an inline fallback, report the live call,
reservation or competing home that exhausted the usable slots. Measure
compiler time separately to check whether fewer pointer-range queries speed
allocation.
