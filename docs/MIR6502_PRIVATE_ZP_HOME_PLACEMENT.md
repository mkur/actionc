# Private zero-page homes in MIR6502

**Zero page is the default memory home for ordinary locals and parameters
whose address the compiler controls.** In optimized MIR6502, enter each such
object into ZP allocation. Choose inline storage for that object when the
pool has no suitable bytes for its lifetime. A value carried entirely in
registers needs no memory home.

The optimized selector checks each ordinary memory home against the selected
instructions, including in leaf routines and the program entry. A call with
unknown returning writes can exclude every pool byte for a crossing value.
[Implementation plan](MIR6502_DEFAULT_ZP_HOME_IMPLEMENTATION_PLAN.md) · [Selector](../src/mir6502/parameter_homes/cross_call.rs) · [TN and TNDBG qualification](benchmarks/6502-default-zp-homes/README.md)

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

## Lifetime-aware spill placement

The current spill allocator reserves every named ZP home across the program
and requires every call in a spill's routine to preserve its byte. The proposed
rule lets a private byte spill share a slot with a named local, a parameter or
another spill when their uses cannot interfere on any reachable execution.
Keep one physical binding per spill. Try another byte or retain the inline
spill when the evidence for sharing is incomplete. [Current allocator and verifier](../src/mir6502/parameter_homes/spills.rs)

- **Within a routine**, track each logical byte from its definitions to its
  last uses on the selected MIR control-flow graph, including joins and loop
  backedges. A store to either occupant conflicts while the other's value is
  still needed, even if the value being stored is dead. At a last use, account
  for instruction read/write order before reusing the byte. A physical clobber
  cannot serve as a new logical definition. [Spill liveness](../src/mir6502/analysis/home_liveness.rs) · [Source-object liveness](../src/mir6502/parameter_homes/logical.rs)
- **Across routines**, check each call site that can reach either occupant.
  A caller value live across a call must survive argument setup, the callee
  and its descendants, and result delivery. A callee spill can use a caller's
  byte before the caller defines its value or after its last use. The same
  callee reached at another site while that value is live can rule out the
  binding for the entire spill. Sibling routines may share when their
  activations cannot overlap. Use matched contracts for opaque calls. Account
  for callbacks, interrupts and reentry wherever they can overlap a lifetime.
  [Call-site checks](../src/mir6502/parameter_homes/calls.rs)
- **Permanent reservations** include source-fixed and runtime-reserved bytes
  and dedicated homes for `OWN` or inferred persistent values that must
  survive between invocations. Continue checking address exposure, explicit
  numeric accesses, scratch and both lanes of a word. Every occupant of a
  proposed byte must pass its own check.

Discovery and final verification apply the same interference rule. After
expansion and spill remapping, rebuild the proof from the final instructions,
bindings and call contracts. Recheck explicit observations of the old inline
addresses and the proposed ZP byte against both layouts. Keep the existing
whole-image cost check and rollback. [Placement and replay](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#loop-carried-compiler-spills)

The [spill placement implementation plan](MIR6502_LIFETIME_AWARE_SPILL_PLACEMENT_IMPLEMENTATION_PLAN.md) covers cross-routine reuse first, followed by sharing within a frame and TN qualification.

## TN examples

- `PopUp` saves `d = c` before calling `Range(..., limit, @c)` and compares
  `c` with `d` afterwards. Under the write-through contract, both
  conditional updates in `Range` target the same byte of `PopUp.c`. Taking
  `@c` keeps `c` in addressable storage. Its old value must reach `Range`,
  and when `FindItem` finds no match, the comparison must see any update
  alongside the original value in `d`. This write alone leaves `d`, `mp`
  and `limit` eligible for ZP. Each still needs free bytes
  across argument setup, `Range`'s own homes and scratch, and the opaque
  `Getchar` call. On both TN runtimes, `d` takes `$EF`. The word `mp`
  lacks a free pair and `limit` competes with `d` for the remaining byte. [PopUp](../samples/tn/modern/LIB.ACT#L923) · [Write-through contracts](6502_SHARED_CALLEE_FACTS_DESIGN.md#write-through-contracts)
- **Spill-sharing acceptance case.** `PopUp` reaches `ArenaCursor` through
  `Push` and `SavePos` before `d = c`, and through `RestorePos` and `Pull`
  after the last use of `d`. Those sites allow the cursor temporary to reuse
  `$EF`. Check the directory-sort path independently. Its `gap` spill spans
  loop backedges and must survive `Fnamecmp`. As a rejection control, add a
  fixture where the caller invokes the same spill writer once before defining
  its value and once while it still needs it. The latter site must block sharing.
  On cartridge and standalone, retain the menu behavior and sorted directory
  order, and measure whether both spills recover the 12 bytes they previously
  saved while `PopUp.d` keeps its ZP home. [Arena routines](../samples/tn/modern/LIB.ACT#L106) · [Sort](../samples/tn/modern/TN.ACT#L181) · [Measured spill costs](benchmarks/6502-tn-home-breakdown/README.md#measurement)
- `Copy.mem`, `len` and `files` survive `Tag(j)`. Its reviewed
  `Store(v(i), value)` writes a directory row and uses `$A0/$A1` as scratch.
  Reserve the scratch and other callee homes. The row's numeric address is
  unnecessary for placing the three private locals. Calling the same writer
  with `$00A2` also occupies `$A2`. Passing `@mem` exposes `mem` itself.
  [Tag](../samples/tn/modern/TN.ACT#L355)
- `Close` preserves the source-home pool when TN's reviewed error handler is
  installed. `Open` and `Bget` use `$A1` as scratch, and their callers stage
  an argument at `$A3`. In `Bget(1, mem, len)`, CIO writes through the address
  held in `mem`. The compiler controls the separate home storing that address.
  The allocator currently leaves `Copy.mem`, `len` and `files` inline because
  it cannot determine the destination when the call loads the advancing cursor
  from the local `mem`. An explicit one-byte destination at `$EC` excludes
  `$EC` while allowing adjacent pool bytes at that call.
  [File routines](../samples/tn/modern/LIB.ACT#L559) · [Copy loop](../samples/tn/modern/TN.ACT#L910)
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

### Retained local values at program entry

A local that can be read before its first definition needs the value left by its
previous invocation. A dedicated home reserves the entire byte or word across
the image, including compiler scratch, ABI lanes, and other routines' locals.
MIR6502 currently establishes this for a closed native program with a
parameterless, nonreturning entry. Every call stays in the image. Opaque code,
unresolved indirect memory effects, calls back into the entry, and observable
external entry points prevent this proof. Such locals retain inline storage.

For an accepted dedicated home the emitter writes its initial bytes in the
program entry prologue. The first basic block follows the prologue, so a loop
back to the first block preserves the value. The selector supports scalar
constant initializers and zero-filled storage. The image comparison includes
the initialization instructions and the removed inline data.
