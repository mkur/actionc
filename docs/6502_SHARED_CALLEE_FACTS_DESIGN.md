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
complete table and scratch ranges. A separate review version admits bounded
buffer declarations and local conditional branches. Each declaration names a
pointer formal, a byte-count formal, its zero convention and the direction of
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
