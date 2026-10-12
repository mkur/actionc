# Call predicates

[Implementation plan](../call-predicate-implementation-plan.md).

Stage 0 freezes compiler `a15f9f36` and the executable SHA in
[stage0.json](stage0.json). The generic `Forward` still needs five spill bytes.
Both intact roots match the address-value qualification images, including
classic. Actual LF/CRLF compilations produce identical images in both runtimes
for the fixtures and in cartridge mode for both roots.

The new `call_predicates` VM target checks original snapshots, effect order,
normalization and intact-root `SetTagged` status, bitmap guards and counts.
Together with `tn_codegen_reproducers`, seven tests pass and one new regression
is temporarily ignored: MIR6502 reloads an earlier argument after a later call
mutates its parameter through a passed address. Classic preserves the snapshot.
This correctness defect must be repaired before enabling capture removal.

The repair freezes typed address exposure before temp cleanup. An internal
callee summary cannot authorize moving a home load across a call once that
home's address can escape. All four `call_predicates` tests now pass, including
the unignored alias regression, in both runtimes with actual LF/CRLF inputs.

Direct TN/TNDBG MIR `SetTagged` at ordinal zero costs 842 cycles for setting 0
and 930 for settings 1/2/255. Out-of-range ordinal 20 costs 516/518 cycles.
These vectors execute the wrapper; other directory benchmark cycles cannot be
attributed to it. Cycle observations are not correctness assertions.

Large images, listings, raw MIR and logs live in ignored
`target/tn-call-predicates/stage0`. One debug-free, nonincremental cache is shared
by compiler and VM checks.
