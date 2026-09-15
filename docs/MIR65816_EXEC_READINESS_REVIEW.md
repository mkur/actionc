# MIR65816 Exec readiness review

Date: 2026-09-15. Compiler baseline:
`c73818d91a2b458aa0d65fcb30d2e3ca844699ec`.

This reviews the [Exec readiness requirements](MIR65816_EXEC_READINESS_REQUIREMENTS.md)
against the current compiler. The working tree also contains unrelated sample
and documentation changes. No compiler implementation changed during this
review. The findings and proposed sequence below do not constitute executable
65816 qualification.

The subsequent [CPU test drive](../tools/vm65816-runtime-tests/README.md) records
independent jgenesis execution and its limitations. It does not resolve the
compiler gaps or establish emitted-code readiness described in this review.

The later [X65 execution checkpoint](MIR65816_CPU_EXECUTION_CHECKPOINT.md)
records the C qualification, Rust port, VM integration and Altirra comparison.
Native compiler emission and the Exec acceptance gates remain pending.

The [lowering contract](MIR65816_LOWERING_CONTRACT.md) now implements the four
information-preservation corrections identified below, with permanent regression
tests. The physical ABI decisions and emitted execution remain pending.

## Assessment

The requirements set the right readiness standard for Exec. Reentrancy,
interrupt safety and executable evidence should drive the work. The proposed
two-context qualification harness can prove those properties before building
a scheduler, allocator or message system.

The shared language and NIR foundations are useful. MIR65816 currently stops
at lowering and frame planning, however, and needs instruction selection,
allocation, emission, linking and independent
execution. Keep all six acceptance gates before declaring the compiler ready
for Exec implementation.

## Lowering gaps identified at the reviewed baseline

| Area | Baseline issue | Implemented correction |
| --- | --- | --- |
| Signed comparisons | Signed and unsigned comparisons became identical MIR operations. | Comparison signedness is retained from the operand type. |
| LONGINT arithmetic | Binary lowering recognized only INT as signed. LONGINT division was marked `signed=false`. | Signedness is retained for every integer width. |
| Static data | Lowering skipped zero-fill globals, assigned byte alignment to initialized globals, and omitted stable storage IDs and descriptor cells. | Explicit data identities, placement, extents, alignment, initialization and descriptor/backing objects are retained. |
| Relocations | Byte-selected address relocations lost their byte index. | Low/high/bank selection, addends and address spaces are retained; NIR rejects invalid selectors. |

The relevant code is in [operation lowering](../src/mir65816/lower.rs),
[data lowering](../src/mir65816/data.rs) and the
[MIR definitions](../src/mir65816/mod.rs).
An emitter must not recover discarded information from source syntax or
display names.

The preliminary NIR-to-MIR65816 correction slice is covered by
[permanent contract regressions](../tests/mir65816_contract.rs), including
negative diagnostics. This establishes retained lowering facts; executable
correctness still requires emitted-code tests.

### Arithmetic probes at the reviewed baseline

Two separate programs differing only in their operand declaration:

```action
INT a,b
BYTE result
PROC Main()
  result=a<b
RETURN
```

Replacing `INT a,b` with `CARD a,b` produced an identical
`Mir65816Op::Compare`. The comparison representation retains width and the
comparison operator, but not signedness.

```action
LONGINT a,b,result
PROC Main()
  result=a/b
RETURN
```

At the reviewed baseline this produced a four-byte `Mir65816Op::Binary` with `operation=Div` and
`signed=false`. These observations came from temporary probes through the
real parser, semantic analysis, NIR lowering and `mir65816::lower_program`.
They are lowering findings, not executed machine-code failures. The probes
were not added as permanent regression tests during that review; the subsequent
lowering correction now covers both cases permanently.

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

1. **Publish ABI v1.** The identified information-loss gaps are corrected.
   Specify assembly-visible layouts and conventions on top of the retained facts.
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
