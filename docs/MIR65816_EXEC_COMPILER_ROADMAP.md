# Exec816 compiler improvement roadmap

Exec816 is the primary application for the native 65816 backend. Its kernel,
services, drivers, filesystem and shell should guide compiler development and
provide the main evidence that generated code is becoming smaller, faster and
more economical in memory use.

The central direction is to improve code quality across complete routines and
the complete application. Existing native instruction selection and verified
emission provide a foundation; the next stage should make their benefits apply
consistently to the record, pointer, control-flow and call patterns used throughout
Exec816. Improvements must remain general compiler capabilities that also help
other programs.

## Goals and priorities

Compact release code is the primary optimization objective. Track execution
cost as a second objective, with particular attention to kernel and driver
latency and interactive responsiveness. Evaluate memory use alongside both:
stack depth, direct-page reservations and per-task costs matter in a multitasking
system.

The recorded Exec816 release objective is a maximum of 256 KiB of loaded code
and initialized data with stack guards disabled. Judge progress against an actual
packaged release build, including linked platform assembly. Keep separate
accounting for total RAM, zero-fill, stacks, heaps and distribution-file size.
Measure the guarded debug profile independently.

Correctness, reentrancy, interrupt safety and ABI compatibility are acceptance
conditions for every direction below. Size savings must have an understood
execution-cost tradeoff; a hot-path regression needs an explicit decision.

## Development directions

| Order | Direction | Desired outcome |
| --- | --- | --- |
| 1 | Record and memory code quality | Record access, pointer traversal, embedded data and aggregate operations have predictable costs in ordinary Exec816 routines. Efficient handling extends to realistic combinations of these constructs. |
| 2 | Value placement across complete routines | Values remain available efficiently through branches, loops and joins. Registers, direct page and stack storage are planned together, with clear ownership and lifetime rules. |
| 3 | Calls and runtime interfaces | Routine composition, service boundaries and helper use impose proportionate overhead. Good code quality survives frequent calls and mixed compiler/assembly interfaces. |
| 4 | Complete application footprint | Compiler and linker decisions improve the loaded Exec816 image while preserving its selected functionality. Code, initialized data and runtime memory costs are visible at module and application level. |
| 5 | Sustained execution performance | Workload measurements guide improvements to frequently executed paths and latency-sensitive services. Static size and runtime behavior are evaluated together. |

The first two directions belong together. Record-intensive routines depend on
how pointer and scalar values are managed throughout their control flow. Treat
this combination as the main near-term investment, measured across multiple
Exec816 subsystems.

Call efficiency is the next structural priority because it affects the cost of
building Exec816 from reusable routines and services. Begin within the current
public ABI. Any future ABI evolution requires a separate design decision,
migration plan and application qualification.

The [integrated record/placement qualification](benchmarks/65816-record-placement-stage7/README.md)
supports that next direction with measured application benefits and preserved
native resource contracts. The joint plan's final size/traffic targets and the
unchecked hosted provider gate remain obligations. Carry those targets into the
next assessment rather than treating the placement foundation as complete release
readiness. Keep hosted fixture ownership and release packaging work explicit
alongside compiler development.

Application footprint and execution performance should receive continuous
measurement from the start. Increase investment in these directions as the
routine-level work matures and measurements identify the remaining application
constraints. Larger compiler work should follow demonstrated costs and the
facts needed to handle them safely.

## Compiler foundations

Strengthen the shared facts needed by these directions: value lifetimes,
storage identity, memory effects, control flow and callable contracts. Prefer
consistent analyses and allocation policy over accumulating separate eligibility
rules for narrowly shaped routines. Keep the existing verified state and emission
machinery authoritative, and monitor compiler time and memory use as analysis
scope grows.

Preserve the architectural boundary: SemIR owns language meaning, NIR owns
normalized typed computation and storage facts, MIR65816 owns target strategy,
and emission owns instruction writing and final artifact metadata. New
capabilities should consume verified facts at the appropriate layer.

## Measurement and application qualification

Maintain both a frozen Exec816 workload for compiler comparisons and a current
application build for release readiness. Record compiler, source, runtime and
layout identities so application growth can be distinguished from compiler
changes. Historical measurements retain their original scope and baseline.

Use a compact scorecard for each development checkpoint:

- Loaded release code and initialized data, with module contributions.
- Execution cost and responsiveness for representative kernel, driver,
  filesystem and shell workloads.
- Stack requirements, direct-page use and per-task memory costs.
- Build cost and reproducibility, including supported host platforms.
- Native backend execution results and hosted Exec816 behavior.

Keep focused compiler tests and independent native execution evidence alongside
application tests. Run the affected backend qualification at integration
milestones, and qualify the actual measured Exec816 artifact before declaring
an application milestone complete. Interrupts, task switching, banked memory,
relocation and assembly interfaces remain part of that evidence. VM results and
hardware results must retain their respective scopes.

## Choosing development work

Choose each implementation plan from a fresh Exec816 assessment. Prefer work
that addresses a recurring cost across representative routines, has a clear
compiler-layer owner, and can demonstrate an application benefit within a
bounded implementation scope. Reassess priorities after each substantial
milestone instead of retaining a fixed list of instruction-level candidates.

This roadmap defines development directions. Detailed designs, individual
changes and their qualification records belong in the corresponding
implementation plans.

The joint [record and value placement plan](MIR65816_RECORD_VALUE_PLACEMENT_PLAN.md)
addresses directions 1 and 2, including their compiler foundations and staged
Exec816 measurement and qualification.

The existing [lowering contract](MIR65816_LOWERING_CONTRACT.md),
[emission contract](MIR65816_EMISSION_CONTRACT.md),
[physical ABI](MIR65816_PHYSICAL_ABI_V2.md) and
[native direct-page contract](NATIVE_DP_PARTITION.md) define the current
boundaries. The [code-quality plan](MIR65816_CODE_QUALITY_PLAN.md) retains
historical development evidence; the
[latest BYTE arithmetic measurement](benchmarks/65816-byte-arithmetic/results.json)
is a recorded compiler checkpoint, with its workload and qualification scope.
