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
