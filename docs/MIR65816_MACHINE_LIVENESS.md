# Native 65816 register and flag liveness

Slice 5 of the [implementation plan](MIR65816_ANALYSIS_REWRITE_IMPLEMENTATION_PLAN.md)
adds read-only physical register-lane and independent N/Z/C/V liveness. It uses
the selected CFG, central instruction effects and shared backward solver,
adapting MIR6502's machine-liveness transfer. Selection and allocation are unchanged.

## Contract

Each state contains A/X/Y bit masks, independent flags, and observed environment
requirements. Queries expose the low/high lanes of each register. Joins take
unions. The transfer removes definite writes and definite register/flag
clobbers, then adds reads; call inputs therefore remain live before clobbers.
This describes demand for old values, not definedness of arbitrary call results.

A8 writes preserve hidden A-high. X8 loads/transfers and narrowing account for
zeroed high index bytes. Carry-in arithmetic, RMW carry chains, compare/branch
flags, INX/TXA and full-width TSC/TCS use the same physical effects as emission.
Native return boundaries use `ResultLocation`, including zero extension of byte
and three-byte results. Fault exits conservatively observe all registers/flags.

S, D, DBR, PBR, E/M/X, decimal, I and PC effects remain protected even where
their old values are mathematically dead. X reservations and compiler witnesses
remain additional obligations. No dead-register or dead-flag exception is enabled.

The immutable analysis snapshot supplies `machine_live_before/after` with
checked owner, generation, bounds and reachability, alongside existing home and
definition queries. The forward tracker remains authoritative for its current
omissions; these facts alone do not authorize a rewrite.

## Qualification

Ten new unit tests cover independently authored diamonds/loops, independent
flags, byte lanes, narrowing, carry inputs, compare/branch, INX/TXA, TSC/TCS,
call input/clobber ordering and every native result class. The
[inventory](benchmarks/65816-analysis-rewrite/machine-liveness-summary.json)
queries all 6,939 reachable sites in 28 raw/optimized corpus selections.

Independent [VM perturbations](benchmarks/65816-analysis-rewrite/machine-liveness-perturbations.json)
run raw/optimized code with both incoming I states. Forty declared-dead lane/flag
perturbations preserve the result and complete subsequent memory read/write
events. Sixteen declared-live accumulator/carry/branch-flag perturbations change
the defined result. These representative probes supplement the central effects'
independent instruction/VM coverage; they are not a general equivalence proof.

The [qualification record](abi/action65816-machine-liveness-qualification.json)
captures source hashes, 157 native library tests, 61 affected root integration
tests and eight scoped native tests in each host profile. Existing selected-CFG
and instruction-effect artifacts remain equal. There is no production recording
or emission change, so the full corpus execution gate and isolated CRLF rebuild
are reserved for authoritative replay in slice 6. No NIR, semantic, runtime or
fixture-text handling changed. Compile-time overhead remains unmeasured.
