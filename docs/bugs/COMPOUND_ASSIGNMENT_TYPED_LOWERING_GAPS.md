# Compound assignment loses the operation's type

Status: open; discovered during embedded-record-array backend validation.
Reproduces with ordinary named arrays, without the experimental capability.
Public embedded-array syntax remains disabled.

## Confirmed reproducers

```action
BYTE ARRAY items(2)
PROC Main()
  items(1)==*3
RETURN
```

MIR compilation fails NIR verification: `integer multiplication must produce
cartridge-compatible INT`. The compound operation is incorrectly typed BYTE
instead of computing the normal multiplication result and narrowing on store.

```action
BYTE ARRAY items(2)
BYTE result=$0600
CARD FUNC Divisor() RETURN(256)
PROC Main()
  items(1)=13
  items(1)==/Divisor()
  result=items(1)
RETURN
```

The expected result is zero. Classic produces zero; MIR produces `$FF` in both
runtimes because its BYTE operation loses the divisor's high byte. These are
shared compound-operation defects, not inline-array address failures.

Two explicitly named characterization tests in
`src/codegen/semir/array_execution_tests.rs` retain the observed failures.
Their passing status records a known bug; it does not count these sources as
successful cross-backend semantic executions. Replace those expectations with
the correct oracle when the defects are repaired.

## Cause and next slice

`SemStmt::CompoundAssign` retains the target and RHS types but has no separate
normalized operation-result type. `NirLowerer::compound_or_unsupported` uses
the target type for the binary operation and its result. That is insufficient
when ordinary binary typing widens the computation or mandates an INT product.

Slice 4c of the embedded-array plan should:

1. Reuse SemIR's ordinary binary typing/coercion rules for compounds. Carry the
   operation type and final store conversion explicitly; do not reconstruct
   language semantics in MIR or add operator-specific array selectors.
2. Lower the typed operation followed by the required store conversion in NIR.
   Keep the destination address captured once across RHS calls.
3. Audit old-value load ordering: current NIR produces RHS values before loading
   the target, whereas classic's captured compound path saves its old value
   first. Resolve the language contract using existing tests/original-cart
   behavior before changing it; add a RHS that modifies the same target.
4. Turn the two characterizations into oracle regressions and extend the matrix
   across named arrays, pointers and embedded fields; BYTE/INT/CARD operations;
   widening/narrowing, calls and both runtimes. Keep Compatibility policy
   separate from the experimental modern extension.

Complete this follow-up before public embedded-array enablement. No new
optimizer or per-benchmark specialization is indicated.
