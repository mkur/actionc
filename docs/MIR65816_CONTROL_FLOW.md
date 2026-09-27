# Native 65816 width and control-flow results

The [3a–3c implementation plan](MIR65816_CONTROL_FLOW_IMPLEMENTATION_PLAN.md)
is complete in three independently qualified slices. Public ABI v1, image v3,
o65 profile v1, stack guards and allocation remain unchanged.

## 3a: checked MIR-entry width omission

Implemented in `55579fd` on 2026-09-21. Reachable MIR entries omit redundant REP
only under a native A16/X16 body contract whose complete predecessor obligations
are checked before finalization, including late backedges. All joins retain
value/flag/home/forwarding barriers. Internal labels and unproved/dead entries
retain explicit mode requests. See the [emission contract](MIR65816_EMISSION_CONTRACT.md).

The [frozen inventory](benchmarks/65816-control-flow/3a/baseline.json) selects
60 REP instructions in the 28 counted Action streams. Independent execution
reaches them 944 times per incoming I state, saving exactly 2,832 cycles across
the vectors. The [delta checker](benchmarks/65816-control-flow/3a/delta.json)
validates complete instruction streams, remapped instruction/fusion/copy/forwarding
counts and every measurement field; all stack/DP traffic, frames, guard costs and
peaks are unchanged. The separate 24-routine boundary snapshot changes only 43
MIR-entry REP instructions and the resulting labels, fixups, PER sites and spans.

| Kernel | Mode | Bytes before / after | Cycles before / after | Stack peak |
| --- | --- | ---: | ---: | ---: |
| identity(13) | raw / optimized | 61 / 59 | 66 / 63 | 4 |
| sum_loop(13) | raw | 164 / 158 | 1,767 / 1,683 | 14 |
| sum_loop(13) | optimized | 140 / 132 | 1,395 / 1,308 | 16 |

Validation: 54 emitter unit tests, 59 affected compiler integration tests,
89 native tests in each host (the new independent REP/flag test ran separately
in debug), 24 Python comparison tests, matching LF/CRLF corpus builds and an
isolated CRLF boundary-snapshot rebuild. Debug/release share 374 identical native
artifacts and identical 264 corpus records, each executed with both I states.
Native coverage includes calls, aliasing, serialized/relocated o65, IRQ/NMI and
stack faults. The known optimized vbcc unlink vector-0 failure remains explicit.

The [qualification record](abi/action65816-control-flow-3a-qualification.json)
binds the results to source, tools, saved artifacts and manifests. The measured
[snapshot](benchmarks/65816-control-flow/3a/after/tables.md) in
`target/control-3a-after` is the immutable baseline for slice 3b.

## 3b: terminal fallthrough after edge copies

Implemented in `7a2f616`. Terminal edges omit JML only when their successor is
the immediately following MIR block. All direct/staged/mixed copies execute
first. The tracker closes the logical path and requires that exact next binding;
width policy and value/flag barriers remain unchanged. Earlier false arms retain
their jumps. Nonserialized transfer identities keep zero-byte edges visible to
independent proof tooling, including coincident block entries.

The [frozen inventory](benchmarks/65816-control-flow/3b/baseline.json) and
[exact delta](benchmarks/65816-control-flow/3b/delta.json) agree on 20 static JML
removals and 360 removed executions per I state, saving 1,440 cycles. No data
traffic, stack peak, frame, guard, width or existing optimization count changes.
The boundary snapshot removes only 12 terminal JMLs across its 24 routines.

| Kernel | Mode | Bytes before / after | Cycles before / after | Stack peak |
| --- | --- | ---: | ---: | ---: |
| sum_loop(13) | raw | 158 / 150 | 1,683 / 1,627 | 14 |
| sum_loop(13) | optimized | 132 / 124 | 1,308 / 1,252 | 16 |

Validation: 58 emitter tests, 59 affected compiler integration tests, all 90
native tests per host, the existing 24 Python checks, LF/CRLF corpus equality
and an isolated CRLF snapshot rebuild. Both hosts produce identical 264 corpus
records and 374 native artifacts. Updated edge evidence retains direct/staged
copy, both-arm, o65 and IRQ/NMI coverage; independent ca65 execution confirms
the four-cycle saving with all admitted flag combinations and full A values.
The known external vbcc failure remains visible.

See the [qualification record](abi/action65816-control-flow-3b-qualification.json)
and [measured snapshot](benchmarks/65816-control-flow/3b/after/tables.md).
`target/control-3b-after` is the immutable baseline for 3c.

## 3c: short conditional MIR dispatch

Implemented in `fc43602` on 2026-09-22, following the byte-preserving layout
foundation in `80826b6`. A private routine finalizer shortens typed ordinary and
fused MIR dispatches to their original predicate when the signed displacement
fits. Fixed-point selection accounts for each candidate's own shrink and for
other shortened sites. Out-of-range dispatches retain the inverse-branch/JML form.
Internal comparisons, guards, helpers and indirect stubs retain their selection.

One checked offset map updates labels, retained fixups, PER operands, MIR spans,
logical transfers, conditional records and optional trace PCs. Relocation checks
the encoded displacement, overlap, target and bank placement. Local relative
bytes remain invariant at accepted o65 placements; the serialized formats and
relocator protocol are unchanged.

The [frozen inventory](benchmarks/65816-control-flow/3c/baseline.json) and
[exact delta](benchmarks/65816-control-flow/3c/delta.json) agree on 12 static short
sites across the 28 Action streams, removing 320 executed JMLs and saving 1,050
cycles per incoming I state across the corpus. All data traffic, frames, guards,
stack peaks and existing optimization counts remain unchanged. The boundary
snapshot shortens eight dispatches across 24 routines; every byte and metadata
offset was checked against an independent transformation of the previous output.
This is an intentional machine-code/offset change, with no NIR contract change.

| Kernel | Mode | Bytes before / after | Cycles before / after | Stack peak |
| --- | --- | ---: | ---: | ---: |
| sum_loop(13) | raw | 150 / 146 | 1,627 / 1,587 | 14 |
| sum_loop(13) | optimized | 124 / 120 | 1,252 / 1,212 | 16 |

Validation: 65 emitter/proof tests, 60 affected compiler integration tests,
all 92 native tests in each host, 24 Python comparison tests, five disassembler
tests, LF/CRLF corpus equality and an isolated CRLF boundary-snapshot rebuild.
Coverage includes independent ca65 predicates across a page boundary,
signed displacement boundaries and cascades, generated short/long fallbacks,
banked images, two o65 placements, remapped indirect PER continuations, calls,
aliasing, IRQ/NMI suspension and stack faults. Both hosts share 374 identical
saved native artifacts and identical 264 comparison records, each with both I
states. Their 416 compiler/fixture input hashes match the implemented source.
The known optimized vbcc unlink vector-0 failure remains explicit in both hosts.

See the [qualification record](abi/action65816-control-flow-3c-qualification.json)
and [measured snapshot](benchmarks/65816-control-flow/3c/after/tables.md).
`target/control-3c-after` is the new immutable quality baseline. Across all three
slices, optimized `sum_loop(13)` falls from 140 bytes / 1,395 cycles to 120 bytes /
1,212 cycles; raw falls from 164 / 1,767 to 146 / 1,587. Stack reads/writes stay at
165/188 optimized and 197/246 raw, with peaks of 16 and 14 bytes respectively.

## Shared return tails

Framed routines may retain one return teardown and redirect other reachable
returns to it. MIR blocks and predecessor obligations remain unchanged; the
shared tail is an internal selected-action label. `PrepareReturnJoin` checks
native A16/X16 body depth with no outstanding pushes, then forgets path-specific
value, stack-equation and X-residency facts without changing hardware registers.
Every incoming edge must still match the label's execution environment, including
backedges emitted after the retained tail. The internal join explicitly restores
A16 permission; it does not inherit the permissions of a verified MIR block.

Result preparation and outgoing call cleanup belong to individual return sites.
The retained tail alone owns frame release and the typed native-return summary.
Jump/source spans and layout are rebuilt by the normal typed replay pipeline.
Sharing is restricted to multiple reachable returns with nonzero frames; its
cost includes the retained join repair and redirected jumps.
