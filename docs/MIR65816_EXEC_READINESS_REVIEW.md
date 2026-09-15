# MIR65816 Exec readiness review

Date: 2026-09-15. Compiler baseline:
`c73818d91a2b458aa0d65fcb30d2e3ca844699ec`.

This reviews the [Exec readiness requirements](MIR65816_EXEC_READINESS_REQUIREMENTS.md)
against the current compiler. The working tree also contains unrelated sample
and documentation changes. No compiler implementation changed during this
review. The findings and proposed sequence below do not constitute executable
65816 qualification.

## Assessment

The requirements set the right readiness standard for Exec. Reentrancy,
interrupt safety and executable evidence should drive the work. The proposed
two-context qualification harness can prove those properties before building
a scheduler, allocator or message system.

The shared language and NIR foundations are useful. MIR65816 currently stops
at lowering and frame planning, however, and needs contract corrections as
well as instruction selection, allocation, emission, linking and independent
execution. Keep all six acceptance gates before declaring the compiler ready
for Exec implementation.

## Gaps in the existing lowering

| Area | Current issue | Required work |
| --- | --- | --- |
| Signed comparisons | Signed and unsigned comparisons become identical MIR operations. | Preserve the operand's signedness so instruction selection can choose the correct comparison sequence. |
| LONGINT arithmetic | Binary lowering recognizes only INT as signed. LONGINT division is marked `signed=false`. | Preserve signedness for every supported integer width. |
| Static data | Lowering skips zero-fill globals, assigns byte alignment to initialized globals, and gives data objects names without stable storage IDs. | Complete the storage model before placement and linking; preserve identity, extent, alignment and initialization. |
| Relocations | Byte-selected address relocations lose their byte index. | Preserve low/high/bank-byte selection explicitly, or diagnose unsupported encodings. |

The relevant code is in [MIR65816 lowering](../src/mir65816/lower.rs):
`lower_op`, `lower_program` and `lower_data_image`. The
[MIR definitions](../src/mir65816/mod.rs) also need to carry these facts.
An emitter must not recover discarded information from source syntax or
display names.

Add an explicit preliminary slice to harden the NIR-to-MIR65816 contract.
Focused regressions should check the preserved facts, including negative
diagnostics where support is intentionally deferred. Parsing or successful
lowering alone cannot establish correctness.

### Confirmed arithmetic probes

Two separate programs differing only in their operand declaration:

```action
INT a,b
BYTE result
PROC Main()
  result=a<b
RETURN
```

Replacing `INT a,b` with `CARD a,b` produces an identical
`Mir65816Op::Compare`. The comparison representation retains width and the
comparison operator, but not signedness.

```action
LONGINT a,b,result
PROC Main()
  result=a/b
RETURN
```

This produces a four-byte `Mir65816Op::Binary` with `operation=Div` and
`signed=false`. These observations came from temporary probes through the
real parser, semantic analysis, NIR lowering and `mir65816::lower_program`.
They are lowering findings, not executed machine-code failures. The probes
were not added as permanent regression tests during this review.

## Physical ABI decisions

The existing frame plans remain provisional. They report zero saved-register
and spill bytes. The current 255-byte check therefore does not establish the
final frame limit. Incoming arguments, return addresses, temporary pushes and
allocation must contribute to the final addressing and stack checks. R5
correctly requires this distinction.

Before instruction selection, settle and publish ABI v1:

- Exact stack layout: argument order, caller/callee cleanup, outgoing argument
  placement, alignment and the fabricated initial task frame.
- Register conventions: precise 24-/32-bit result placement, unused register
  bits, decimal-mode policy, and direct-page register `D` and data-bank register
  `DBR` ownership.
- Scratch ownership: invocation-local storage for surviving values, with
  explicit rules for temporary direct-page storage and interrupt re-entry.
- Interrupt policy: initially keep ordinary IRQ handlers non-nested, with a
  separate, clearly bounded NMI policy. This is a proposed starting policy,
  not an implemented guarantee.

Bank-zero reservations need a concrete shared memory map covering bootstrap,
vectors, stacks and interrupt workspace. Direct-page addressing uses bank
zero; native interrupts transfer execution to bank zero while saving the
previous program bank. These CPU constraints are documented in the
[WDC W65C816S datasheet](https://www.westerndesigncenter.com/wdc/documentation/w65c816s.pdf),
particularly sections 2.6 and 7.11. Compiler scratch ownership and the ABI are
project decisions built on that hardware behavior.

## Recommended implementation sequence

1. **Harden MIR facts and publish ABI v1.** Close the information-loss gaps,
   establish storage and relocation identities, and specify assembly-visible
   layouts and conventions.
2. **Establish minimal emitted execution.** Generalize the image transport,
   select and qualify an independent emulator, and execute basic memory
   operations, arithmetic, branches and far calls from binary artifacts.
3. **Prove invocation isolation.** Implement real frames, recursion, assembly
   interoperability, indirect calls and two simultaneously live contexts.
   Introduce the interrupt harness as soon as real calls and frames work.
4. **Complete the kernel subset.** Add banked pointers, records/arrays, wide
   arithmetic, memory helpers, faults and final stack accounting. Expand the
   interruption corpus as each helper arrives.
5. **Qualify preemption and effects.** Complete instruction-boundary IRQ
   injection, NMI, volatile traces, nested critical sections and all six
   acceptance gates in the advertised compiler configurations.

General multiplication/division can wait as the requirements allow, but
address scaling needed by record arrays belongs in the initial implementation.

The first substantial execution milestone should be two independently stacked
Action! contexts entering the same routine and helper, surviving an
interrupt-driven switch and producing verified results. This is an early
milestone on the route to full G1–G6 acceptance, not a replacement for it.

## Validation recorded during this review

```sh
cargo test --locked --test native_routine_abi --test native_type_surface --test modern_integer_arithmetic
```

All 47 existing tests passed: 20 native routine ABI tests, 24 native type
surface tests and three modern integer arithmetic tests. The separate probes
above nevertheless exposed the signedness gaps; the current assertions do not
cover them.

There is still no emitted 65816 execution evidence. No emulator acceptance
gate, interrupt qualification or real-machine result is claimed here.
