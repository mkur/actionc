# Shared 6502 callee facts for classic codegen and MIR6502

Classic codegen and MIR6502 consume the same typed
call contract. For handwritten assembly, an LLM-assisted review establishes
the facts once and records them beside the routine. The build checks that the
review applies to the selected implementation. Each backend then proves its
own transformation at the call site.

## Who establishes the facts

For **handwritten assembly**, the checked-in review is the authority on
behavior. Assembling instructions, resolving relocations and checking the
identity of the body are compiler responsibilities. Neither backend needs to
interpret opcodes to rediscover what a reviewed routine reads, writes or
preserves. An unreviewed body supplies only independently guaranteed ABI facts.

For **compiler-generated Action! routines**, MIR can derive facts from its
typed operations and control flow. Classic codegen can publish the facts it
establishes for its own generated implementation in the same format. Generated
scratch usage can differ between backends. A physical fact belongs to the
implementation that established it.

SemIR resolves source declarations and symbolic references. NIR carries
reviewed declarations as typed metadata keyed by routine and storage IDs.
Classic codegen receives the same declarations through its semantic input.
A shared 6502 component matches them to the assembled implementation and
exposes one set of queries to both consumers. Register placement, overlap
after layout and instruction selection remain target decisions.

## Opaque reviewed calls at every depth

A matched `reviewed-6502` routine is an opaque call boundary. Classic codegen
and MIR6502 use the same reviewed facts whether they are optimizing a direct
caller or summarizing that caller for its own callers. Match the review to the
selected body, ABI and dependencies before using its claims. At each call
site, check the arguments, result ABI and any declared preconditions.
MIR analyzes the native caller's setup, control flow and uses after the call.
The assembly body's contribution comes entirely from its contract.

In TN's `Print → Putchar → Internal` chain, the call has the following transfer.

| Caller question | Answer for `Internal(ch)` |
| --- | --- |
| What values must be supplied? | The signature places `ch` in A. The matched `abi-inputs-only` review permits arbitrary incoming carry and scratch values. |
| What memory can the call observe or change? | It can read `internalMap[0..3]` and write `$A0`. It preserves `$AC/$AD` and `$AE/$AF` on return. The table remains a program-data read at every depth. |
| Where does the caller get the result? | On every normal return, the typed BYTE result is in `$A0` and `returns A=$A0` makes it available in A. The complete read footprint excludes the old `$A0`. A subsequent load by `Putchar` therefore consumes `Internal`'s result. |

Apply input and memory observations before providing the result on the
normal-return edge. A possible write alone cannot establish a result. Equality
between A and `$A0` alone also permits a routine to load the old slot and
return it unchanged. If a contract permits reading the old slot, retain that
dependency. A caller-visible result guarantees a value at the boundary. An
optimization that requires a physical store on every path needs evidence for
that stronger property. [TN implementation and callers](../samples/tn/modern/LIB.ACT)

For zero-page placement, `Print` inherits the obligations and effects of the
native `Putchar` wrapper, including its own writes. Calling through the wrapper
adds no carry requirement for `Internal`. The optional `entry-reads abi`
serves queries about physical reads before definition. A failed match leaves
the assembly opaque. Only independently established guarantees, such as the
argument layout, remain available. [Annotation semantics](ACTIONC_ANNOTATIONS.md)

MIR6502 applies this transfer to direct calls and through acyclic native
wrappers. When a reviewed footprint refers to a table, the analysis uses the
candidate image layout to check overlap with machine bytes. If disjointness
is unproved, the caller retains a possible read of the incoming byte unless
the review independently excludes that entry read. [Entry analysis](../src/mir6502/analysis/known_callees/entry_inputs.rs)

TN's `CalcAdr` review declares `defines-on-return $E6-$E7`. NIR carries
these fixed-byte definitions as routine metadata. MIR6502 applies the
definitions after incoming reads and propagates them through returning paths
of native wrappers. The screen pointer used by `Putchar` is therefore defined
inside the call. Home selection uses the native screen
store's ordinary pointer effect when checking a private caller object. The
reviewed writes to `$A0` and `$E6/$E7` still exclude overlapping homes.
[Caller-home design](MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#ordinary-pointer-writes-across-calls) ·
[TN qualification](MIR6502_CROSS_CALL_POINTER_WRITES_IMPLEMENTATION_PLAN.md#tn-qualification)

## The contract for Ascii

| Caller question | Reviewed answer |
| --- | --- |
| What must the caller supply? | The declared `CHAR ch` under the ordinary Action! ABI. The signature places it in A. Either incoming carry value works because the mask discards the carry bit introduced by the rotates. |
| Which program data may it read? | One byte of `asciiMap`, at an index from 0 through 3. The range refers to the declared table and follows its relocation. |
| Which program data may it write? | Only the byte at `$A0`. The leaf returns with an ordinary `RTS`. |
| What survives a normal return? | The incoming contents of `$AC/$AD` and `$AE/$AF`. |
| Where is the result on return? | A and the byte at `$A0` both hold the result. |

Each answer stands on its own. An omitted memory footprint is unknown. An
explicit empty footprint means no program-data access in that direction.
A complete may-write set bounds *every* possible write. Listing one clobber
alone leaves other addresses open. Preservation describes the value on return
and allows temporary changes during the call. Reads and writes used for
deleting or moving operations cover the entire invocation, including any
observable I/O. The ordinary call and return account for implicit stack
traffic. [TN assembly](../samples/tn/modern/LIB.ACT)

MIR6502 also uses the reviewed entry when laying out the callee. A matched
ABI-only entry can keep its parameter widths and incoming lanes without
reserving inline slots. The whole-entry review excludes assembly operands
that name a parameter. A data relocation that exposes a frame address keeps
that frame allocated. For `Ascii`, the caller supplies `ch` in A and the
callee writes its result to `$A0`. Both transfers use the ABI directly.

## What the caller still has to prove

Consider `s(i) = Ascii(s(i))` with the destination pointer already in
`$AC/$AD`. The caller can reuse the pair after the call if it proves that
the pair still denotes the intended destination. The preservation fact
establishes the callee's part of that proof. The caller can store the returned
byte straight from A if its own index setup leaves A intact.

Deleting an earlier store to a spill is a different decision. It also
requires proof that the callee cannot read that spill and that the value
has no other live use. The bounded table read supplies the callee's part
once layout establishes disjointness. Similarly, allowing arbitrary incoming
carry satisfies the call's entry obligation. An optimizer reasoning about
which flags an instruction physically reads still needs its own evidence.

## Review validity and adoption

Bind the review to the target ABI, assembled body and typed relocation
dependencies, including each referenced object's extent and backing where
relevant. A layout-only move can retain the review when those assumptions
still hold. A changed body, incompatible ABI or stale dependency makes the
affected facts unknown in both modes. Matching the fingerprint establishes
which implementation was reviewed. An independent VM test checks the claim
itself. Deterministic checks run on each build and edits that invalidate a
review require a fresh review before its facts can be used again.

Direct calls to straight-line leaves such as `Ascii` and `Internal` can use
complete table and scratch ranges. Full reviews can identify forward conditional
branches to local instruction boundaries and table references with an in-bounds
constant offset. A separate review version admits bounded buffer declarations
and backward local branches. Each buffer declaration names a pointer formal,
a byte-count formal, its zero convention and the direction of
access. The review establishes the bound. MIR instantiates it with the current
physical arguments or a coherent pointer interval and checks the resulting RAM
range. A numeric caller-supplied pointer also contributes its possible span to
the physical storage inventory. Unresolved indirect effects remain conservative
at the call. Classic can use the fixed facts independently and treats an
unresolved buffer write as a possible memory clobber.

Legacy entry-only fingerprints establish ordinary argument sufficiency. A full
review can also explicitly certify physical entry reads through `entry-reads
abi`. An unmatched review on source assembly supplies neither guarantee.
[Annotation syntax](ACTIONC_ANNOTATIONS.md) · [MIR6502 call contracts](MIR6502_CALL_FACT_CONTRACT_DESIGN.md)

Qualify the first slice in both backends. Compare the published facts for the
same assembly, run all 256 input bytes with both carry values, and trace data
accesses against the declared table and return slot. In a caller that already
holds the row pointer in `$AC/$AD`, inspect whether the pair survives the
`JSR` and the result can be stored directly from A. TN's
`FormatDirectoryRow` exercises this pattern with `Internal`. Change the body
or table extent to exercise stale review handling. Withhold the read footprint
to check that a potentially observed spill store survives even when pair
preservation is known. [TN callers](../samples/tn/modern/TN.ACT)

The [implementation plan](6502_SHARED_CALLEE_FACTS_IMPLEMENTATION_PLAN.md) sequences
the shared entry check, reviewed memory facts, caller changes and TN qualification.


## Reviewed calls and indirect destinations

A reviewed assembly body can delegate to another reviewed implementation using
a typed direct call. Each call operand binds the target identity, its current
implementation and its declared facts. A change invalidates enclosing reviews
in both backends. An unresolved call or a recursive dependency cannot supply
a review. Stack balance and the agreement between each caller and callee are
part of the review. An opaque body can tail-jump to a reviewed routine under
the same identity check. Its references to ordinary uninitialized global
storage bind the backing kind, name and byte extent.

TN's `Close`, `Xio`, `Open`, `Bget` and `Bput` accept ordinary ABI arguments.
The block transfers pass the buffer and 16-bit length to `_BlockIO` under
its private convention. All five leave reads and writes open because both
the installed device and the error handler can modify memory before returning.
For TN's selected entry, `Close` forwards the installed handler's
normal-return preservation for the source-home pool. `Open` and `Bget` use
`$A1` as scratch. `Bget` also excludes the transfer destination specified
by its buffer address and 16-bit byte count. Its shared review keeps that
conditional guarantee distinct from unconditional byte preservation, so a
consumer can use it when it knows the call's destination.
These reviews rely on CIOV using OS scratch outside the pool. The EOF arm of `NavError` preserves the pool,
and the recovery arm discards the caller's stack. SemIR resolves the callback
cell and installation. The shared 6502 review matches the entry block and
projects the same return facts into classic and NIR for that
program. The general memory effects still include I/O and callback activity.
[Installed callback review](ACTIONC_ANNOTATIONS.md#installed-callbacks)

The explicit operand inventory still distinguishes a fixed pointer fetch in
`(zp),Y` from the indexed pointer fetch in `(zp,X)`. The assembler supplies
that distinction in the relocation. [TN file routines](../samples/tn/modern/LIB.ACT)

A caller may undertake a numeric bound on every indirect destination of an
invocation. Fixed writes remain explicit and compose with the indirect range.
The range covers the complete footprint, even for a strided copy or a wrapping
index. A review with those obligations can protect disjoint ZP bytes without
asserting that ordinary memory is unchanged. A normal-return summary for a
native wrapper additionally includes its own generated scratch writes. Native
storage-only loops without a returning path keep their terminal effects in
general rewrite and observation queries.

In TN, `Window` passes `GetImage` the arena allocation for its rectangle.
`SwapScr` passes its full-screen allocation on the first capture and a row
within that allocation on later captures. For `Block` and `PutImage`, the
destination is the screen rectangle computed by `CalcAdr`. [Window and image routines](../samples/tn/modern/LIB.ACT) · [SwapScr](../samples/tn/modern/TN.ACT)

`Push` and `SavePos` use a checked arena copy entry. The unrestricted
`MovePage` entry retains its argument-relative buffer contract. A live caller
home also has to survive outgoing argument stores and spill writes anywhere
in the called routine's subtree. For example, the caller stages the fourth
byte for `PutImage` in `$A3` before entering the reviewed body.

For ZP home selection, a reviewed copy contributes its fixed writes and each
destination span at the call site. In `RestorePos`, `MovePage(@zx,
arenaRestoreCursor, 4)` writes `$005A–$005D` and uses `$A0–$A2` as scratch.
The dynamic source contributes reads. For a potentially nonempty copy, the
write footprint stays unknown if the destination is unknown or the span can
overwrite a fixed pointer or count carrier. Native wrappers also contribute
their own staging and cursor writes.
A write to an in-bounds byte of an ordinary image global, such as
`arenaRestoreCursor`, is disjoint from ZP when the image origin is at least
`$0100`. Fixed-address globals and their aliases use their physical addresses.

## Write-through contracts

A write-through fact bounds the bytes a routine may write through a pointer
argument. Identify the formal parameter, a byte offset from the pointer value
on entry, and the extent. For example, TN's `Range` may write one byte at
offset zero through its third argument, `c`. Either update reads the old
byte, and a call can return without changing it. A consumer must preserve
that incoming value on the no-write path. Fixed writes, other pointee effects
and opaque calls contribute their own facts. [Range source](../samples/tn/modern/LIB.ACT#L779)

Use the same argument-relative fact for native and reviewed implementations.
For reviewed assembly, the existing `writes-buffer 3 bytes=1` clause expresses
the example span. For native Action!, derive it from typed operations. At a
branch or loop, take the union of possible destinations. Repeated stores
through the same entry pointer keep the same extent if the pointer retains
its value on every path to each store, including loop backedges and intervening
calls. A wrapper forwarding its own pointer argument carries the span into
its contract. If a path can redirect a store outside the proven span, widen
the effect to cover that path or leave its destination unknown.
[Reviewed buffer syntax](ACTIONC_ANNOTATIONS.md#reviewing-an-argument-relative-buffer)

At each call, bind the formal to the actual argument. An address of a known
object identifies bytes of that object. A numeric address identifies physical
bytes, including any 16-bit wrap. Combine overlapping spans when arguments
alias. If the actual destination remains unknown, keep the possible write
and require independent evidence that a candidate home is disjoint or
preserved. The allocator checks these writes alongside ABI staging, scratch,
callee homes and opaque normal-return guarantees. General memory effects can
remain open even when the allocator can prove a particular home survives.
[ZP allocation and the PopUp example](MIR6502_PRIVATE_ZP_HOME_PLACEMENT.md#tn-examples)

The [write-through implementation plan](MIR6502_WRITE_THROUGH_CONTRACTS_IMPLEMENTATION_PLAN.md)
covers native inference, call-site checks and TN qualification.
