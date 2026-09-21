# Native 65816 width and control-flow results

The [3a–3c implementation plan](MIR65816_CONTROL_FLOW_IMPLEMENTATION_PLAN.md)
is being implemented in independently qualified slices. Public ABI v1, image v3,
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

Slices 3b and 3c remain pending.
