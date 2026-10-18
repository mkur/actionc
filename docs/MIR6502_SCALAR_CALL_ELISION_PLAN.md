# Scalar capture cleanup across known calls

Implement the second follow-up to constructor storage cleanup in MIR6502.
Keep the existing scalar forwarding and deletion proofs, then independently
cost a stronger observation proof for unread argument captures and pattern
bindings in unexposed frames.

1. Follow every stored byte through the owning routine's CFG until overwrite
   or return. Admit direct calls only with complete existing read/write
   summaries and a bounded audit of the complete callee graph. Check named
   accesses in their owning frames; flat callee-local IDs are not caller IDs.
2. Retain stores/homes around unknown or recursive calls, helpers, machine
   blocks, hardware/absolute memory, pointer accesses, unbounded accesses and
   frame-address/data exposure. Do not infer privacy from compiler provenance.
3. Accept only a smaller emitted image with no affected-routine code/home growth
   or additional occupied zero page, and with final physical alias exclusion.
   Failure retains the previous scalar optimization result.
4. Recognize immutable address constants in scalar argument forwarding. Keep
   their relocation identities and ordinary address-exposure accounting.
5. Add focused proof, rollback and VM regressions; qualify MIR6502 unit,
   integration, fixture and relevant runtime targets. Exercise both runtimes,
   raw/optimized NIR and LF/CRLF parsing. Measure maintained TN/TNDBG and the
   existing menu-variant experiment before committing the implementation slice.

The current TN listings still have unread scalar stores, but `FindItem`/`Ord`
read through pointers and `PopUp`/`Format` expose frame addresses. Those facts
must remain blockers unless a separate bounded pointer/escape proof exists.
This slice does not assume that the entire 85-byte caller overhead is removable.
No semantic, NIR, variant-validation or ABI changes are planned.

## Implementation results

The independent call proof removes unread scalar homes after forwarding,
including when forwarding alone was cost-neutral. It checks both callee body
effects and the caller's argument/result placements. An alias or cost rejection
retains the previously accepted scalar optimizations. Immutable routine/global
addresses and split address lanes can be forwarded without losing relocation
identity. The existing frame-exposure and pointer barriers remain in force.

The focused two-level call example removes a CARD binding home and reduces
raw-NIR emitted code/data from 132 to 118 bytes. With optimized NIR it remains
99 bytes: that pipeline already promotes the binding. Address forwarding is
also covered through relocation, alternate-origin and runtime regressions;
its source-level example has no additional saving over the previous compiler.

Final CLI `mir6502`, cartridge-runtime measurements are byte-identical to
the preceding constructor-payload slice:

| Program | Before XEX | After XEX |
| --- | ---: | ---: |
| Maintained TN | 9,968 | 9,968 |
| Maintained TNDBG | 13,073 | 13,073 |
| Menu-variant experiment | 10,120 | 10,120 |

The experiment still has 152 bytes of total overhead over maintained TN.
Its remaining captures cross pointer operations or expose frame addresses,
so this slice does not remove those stores. Doing so requires a separate
pointer-provenance/escape proof.

Validation passed: 1,121 MIR6502 unit tests (the full 1,120-test backend run
plus the subsequently added relocation regression; two existing tests are
ignored), 96 tests across all 11 `mir6502_*` integration targets, and all 169
fixtures through `actionc-mir6502-sweep --emit-load`. The 51 VM tests cover
scalar captures, constructor storage, copy values, aggregate calls/indirect
calls/fresh initialization, TN dispatch and variant construction/validation/
initialization, with relevant raw/optimized modes and both runtimes. The new
call-elision source is checked with LF and CRLF through the actual parser.
No fixtures or NIR contracts changed.
