# Bounded pointer and escape proofs for scalar captures

Implement a conservative MIR6502 proof that an otherwise unread scalar capture
cannot be observed through a pointer. Keep its allocation initially. This is
compile-time analysis: no runtime checks, source changes, or new NIR forms.
Commit this plan first, then commit each implementation slice separately.

## Scope and evidence

The starting compiler is `11ed1538`. Cartridge-runtime optimized MIR XEX sizes
are TN 9,968 bytes, TNDBG 13,073 bytes, and the menu-variant experiment 10,120
bytes. Existing scalar proofs reject an entire frame when any cell's address
is exposed, and reject indirect callee accesses even when their bounds could
be established from actual arguments.

TN is the first measurement target, but its current blockers exceed simple
borrowing: `Range` calls input code and accesses fixed addresses; `FindItem`,
`Ord`, and `Next` scan variable-length menus through routine addresses. A
routine address does not establish a data object's extent. These cases must
remain conservative. Establish a general optimized-MIR regression with a
bounded object before considering a later extension for menu scans.

## Safety contract

- Identify locals and parameters by both routine and storage ID. Globals and
  statics retain their stable IDs. Never compare bare callee-local IDs with
  caller-local IDs or recover missing facts from SemIR or display names.
- A pointer fact identifies an ordinary object, its extent, and a checked
  offset. Initially offsets are singleton ranges, with constant arithmetic
  only. Every access includes its width; overflow and out-of-object accesses
  fail the proof. Numeric and routine addresses have no object bounds.
- Prove reaching values at the actual use. Start with preceding same-block
  definitions, complete word transfers, and immutable incoming parameters.
  Do not mistake a unique definition occurrence for dominance. Unsupported
  joins, partial address lanes, variable offsets, and lost provenance fail.
- Audit pointer uses as well as dereferences. Returned pointers, pointers
  retained in observable storage, unknown calls, and unproved arithmetic
  cannot establish non-escape. A private temporary pointer home is admissible
  only when all its reads are covered by preceding same-block stores and its
  address cannot escape; do not trust compiler provenance alone.
- Calls use their real ABI parameter order and widths. Only complete direct
  Action bodies with an acyclic call graph can supply a narrower footprint
  than coarse unknown call effects. Audit argument/result placement separately.
  OS calls, helpers, machine blocks, external/indirect calls, absolute/hardware
  accesses, volatile copies, and unsupported effects remain barriers.
- Frames are routine-static. Reject reentry and recursion, data relocations
  exposing audited frames, alias slots, and mutation of parameters used as
  incoming pointer facts. Check the complete audited routine, including uses
  before a candidate store and after a call; return alone does not erase an
  escaped pointer. Limit each audit to 512 operations and 16 call levels.
- Prove no access to a removed capture's bytes. Preserve all other stores,
  calls, reads, evaluation order, validation, and allocations. Check physical
  bounds and disjointness in both baseline and proposed layouts, as well as
  existing absolute/global/local alias exclusions.

## Slice 1: bounded object and value facts

Add an internal analysis for ordinary named objects and checked constant
offsets, with owner-qualified local/parameter identities. Establish a small
same-block value tracker that distinguishes a proved pointer from scalar data
and unsupported provenance. Test local/global/static origins, word access
boundaries, overflow, wrong owners, unknown and aliased objects, redefinitions,
cross-block uses, and unsupported address forms. No optimization consumes it
in this slice; emitted programs remain unchanged.

## Slice 2: bounded access and non-escape summaries

Audit a routine and its direct callees using actual pointer arguments. Return
structured read/write ranges with owning identities, or a conservative
failure. Support bounded scalar loads/stores and nonvolatile aggregate copies,
constant-offset pointer derivation, and carefully audited temporary pointer
homes. Track parameter facts through direct calls; reject escapes, recursion,
unresolved effects, and exhausted budgets. Test transitive calls, ABI mapping,
retained and returned pointers, partial reads, parameter mutation, frame data
exposure, unknown callees, and each boundary. Keep this analysis independent
of existing flat callee summaries and existing optimization proofs.

## Slice 3: store-only capture cleanup and qualification

Add an independently costed consumer after existing scalar cleanup. Retry
existing forwarding, identify captures with only stores remaining, and remove
only stores justified by the new footprint/non-escape proof. Keep their slots.
An analysis, verification, layout, alias, or cost failure preserves the
previously accepted program. Require a strictly smaller emitted image, no
affected-routine code/home growth, and no additional occupied zero page.

Add positive and negative optimization tests and VM regressions covering
bounded reads/writes/copies, repeated calls, scalar and aggregate arguments,
raw/optimized NIR, both runtimes, relocated origins, and LF/CRLF parsing.
Run affected MIR6502 unit/integration tests, its emission fixture sweep, and
relevant scalar/aggregate/variant/TN VM targets. No shared compiler contract
changes are intended; if needed, broaden validation as required by AGENTS.md.
Update the pseudo-machine contract and record measured results here.

Measure the focused example and all three TN builds against the saved starting
compiler. Require a real optimized-MIR size improvement in the focused case;
report TN results even if unchanged. Do not expand this task into variable
scan bounds, general interprocedural alias analysis, capture allocation
removal, or new NIR facts to chase a TN saving.
