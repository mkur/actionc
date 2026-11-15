# TN leaf compiler-facts audit — 2026-11-15

The audit found a **confirmed wrong-code bug in machine-callee N/Z summaries**,
an incomplete indexed-read footprint, and three native leaves whose unknown
reads unnecessarily make their writes unknown. Correctness of the summaries
should come before extending their optimization consumers.

Compiler: `7ab6f840ba0475c5ddde9194c130ee83d96c83d1`, optimized NIR and MIR6502,
ActionCart, origin `$3000`. TN reproduces the qualified **9,791-byte** image
exactly. No production compiler behavior was changed for this audit.

## Scope and evidence

All 101 routines in the selected executable were inventoried. Forty have no
MIR `Call` or `RuntimeHelper` operation. Source and assembly inspection separates
them into:

- **18 emitted, call-free routines:** eight functions, nine returning procedures,
  and the non-returning ArenaStop.
- One empty Error declaration, with no emitted body.
- Fifteen assembly wrappers/dispatchers that do transfer control to other code.
  Getchar uses a manufactured RTS chain, so searching only for JSR misses it.
- Six menu data blocks represented as routines, never executable leaf bodies.

This is an audit of TN's emitted program, including LIB.ACT. It does not inventory
the external Action! cartridge or operating system. A routine inlined away before
the selected program is not an additional emitted leaf.

Artifacts:

- [Compiler facts](facts.compact.json): entry registers/flags/fixed bytes;
  analysis and rewrite exit summaries; logical and resolved memory identities;
  frames and parameter homes; operation effects and unknown-write triggers;
  direct MIR callers and their residence-check results; MIR and assembly.
- [Assembly](leaves.asm): all forty candidates, including wrappers and data.
  The six menu blocks are disassembled data, not instructions to audit as code.
- [Static cross-check](audit.json): direct accesses versus exact summaries,
  indexed/indirect accesses, external transfers and X/Y preservation checks.
- [Generation receipt](receipt.json) and [validation receipt](validation.json).

Facts come from the selected MIR after the normal final-image selection. The
probe calls the actual private analyses, including `analyze_for_rewriting` and
`ResidenceCalls::writes`; it does not reconstruct their decisions from a map.
An additional emission resolves logical storage identities and asserts unchanged
image bytes and relocations. This replay is audit work, not optimizer work.

`declared_effects` is separate from the analysed summary. Native routines often
have empty default declared effects; these do not prove that the routine reads
or writes nothing. Direct caller lists exclude references embedded in assembly.

## 1. Confirmed wrong-code: unknown values become an invented N/Z fact

Position's selected body is:

```asm
STA $FF
STX $FE
RTS
```

The compiler correctly proves X/Y preservation and `A = memory[$FF]`, but also
publishes `N/Z = memory[$FF]`. None of these instructions establishes N/Z.

In [known_callees.rs](../../../src/mir6502/analysis/known_callees.rs),
`record_machine_register_store` uses:

```rust
source == MirReg::A && zn.as_ref() == accumulator.as_ref()
```

At routine entry both options are `None`. Their equality is treated as proof
that A established N/Z, so STA invents a flag provenance fact. Unknown provenance
is not an equal known value. A store can give A a memory identity without giving
N/Z that identity.

The independent [control](position-control.act) calls `Position(0,128)` and returns
one if `kx=0`. The compiler emits:

```asm
LDA #$00
LDX #$80
JSR Position
BNE false_result
```

LDX sets Z=0. Position preserves that flag while storing zero into kx. The BNE
takes the wrong arm. Actual VM execution returned **0 instead of 1**, with kx=0,
ky=128 and balanced stack, in both `optimized()` and `default()` configurations.
Both configurations enable peepholes. See [results](position-control-results.json)
and [optimized assembly](position-optimized.asm).

The same invalid inference occurs in GetD after its unmodelled call to `_Cio`
and subsequent `STA $A0`. GetD is a wrapper, not a true leaf, but the broad
candidate inventory exposes it. No GetD-specific failing runtime case was run.

The bad fact is published by `analyze_for_rewriting` for immutable machine
routines and consumed by the call transfer in
[machine_values.rs](../../../src/mir6502/analysis/machine_values.rs). It is not
merely an inaccurate diagnostic. The control establishes a compiler failure;
this audit does not claim a demonstrated failure in TN's current UI paths.

**Priority:** require known, equal provenance before rebasing N/Z through STA.
Cover an initial store and a store after a call that loses flag provenance,
while retaining valid load/ALU-then-store cases. Do this before broadening reuse.

## 2. Exact indexed-read summaries do not describe all bytes read

Three true assembly leaves publish incomplete `Exact` read sets:

| Routine | Actual indexed reads | Published indexed-read identity |
| --- | --- | --- |
| CalcAdr | `ADC quickmul,X`; `ADC quickmul+24,X` | `quickmul+0` only |
| Internal | `ORA internalMap,X`, with X masked to 0..3 | `internalMap+0` only |
| Ascii | `ORA asciiMap,X`, with X masked to 0..3 | `asciiMap+0` only |

`summarize_machine_instruction_reads` records the base Global/Static identity
for absolute indexed reads. It does not describe the index range. In addition,
`expand_machine_operand` ignores a typed relocation's addend when deriving its
memory identity, explaining why CalcAdr's `+24` also disappears.

These facts are insufficient as complete byte footprints. Current consumers
include private-memory observation checks, so an object-base identity must not
silently mean a proven bounded read. No indexed-read wrong-code execution was
demonstrated here; unlike Position, this finding is a contract/correctness risk.
The affected TN tables are source storage, and existing guards can mask the risk.

**Priority:** make unbounded indexed reads unknown, or represent a genuinely
proved range; preserve relocation addends. Do not infer in-bounds access merely
from the operand naming a global. No incomplete direct write set was found in
this TN census.

## 3. Unknown reads unnecessarily poison writes in three native leaves

The complete explicit data-write footprints of the affected generated bodies are:

| Routine | Compiler write summary | Writes visible in assembly | First residence blocker |
| --- | --- | --- | --- |
| Key | Unknown | `$A0,$AC-$AF` | Unknown callee writes |
| Instr | Unknown | `$A0,$AC/$AD,$E0,$32E1-$32E3` | Unknown callee entry |
| Fnamecmp | Unknown | `$A0-$A3` | Extra entry flags or stack |

None contains an indirect store. Key and Instr encounter `LoadIndirect`;
Fnamecmp also encounters `CompareIndirectBytes`. These operations set
`memory.has_unknown_effects` for an unbounded read. `summarize_op_writes` treats
that broad marker as an unknown write. The existing exception for
`OffsetPointerByIndirectByte` does not cover these forms.

Next is the useful control: it reads through a pointer but already has exact
writes `$A0/$A1,$AC-$AF`. Its selected indirect ALU operation does not trigger
the same broad marker. The assembly Value routine similarly has unknown reads
and exact writes `$A0/$A1`. Thus the analyses can represent the desired separation;
the issue is how operation effects are projected into that representation.

Key has accepted entry obligations and is called by FindItem and PopUp. It is
the immediate blocker encountered by the multiple-leaf placement work. Correcting
its write summary would remove that rejection; savings still require the normal
lifetime, preservation and final-image cost checks. No savings are claimed here.

Instr and Fnamecmp would remain blocked for additional reasons:

- Instr's two-byte `s` parameter is `storage=Array`, not Scalar. Its direct ABI
  is `[Word, Byte]`, but the entry certificate explicitly requires scalar slots.
  The public lowered frame confirms this; the selected frame still has ordinary
  three-byte parameter storage.
- Fnamecmp's entry fact requires C and Z in addition to A/X/Y and `$A3`.
  Its emitted CMP/INC instructions define the flags before their branches.
  The conservative whitelist in `machine_liveness::entry_flag_transfer` does not
  certify these compare and word-update recipes as definite entry-flag kills.
  Sort consequently gets `extra-entry-flags-or-stack` before the write test.

These are independent conservative gates, not justification to bypass entry
validation when fixing write summaries.

## Complete census of actual leaves

E means the compiler publishes Exact, U means Unknown. E* marks the incomplete
indexed-read facts above. X/Y means the incoming index registers are preserved;
`—` means no such fact is published, not necessarily that both are clobbered.
Entry `unknown` means no complete entry certificate. No flags are required by
the known entries except the spurious C/Z requirement on Fnamecmp.

| Routine | Bytes | Body | Reads / writes | Preserved | Entry requirements | Audit |
| --- | ---: | --- | --- | --- | --- | --- |
| MovePage | 19 | ASM | U / U | — | A X Y, `$A3/$A4` | Unbounded copy writes justify U; X is actually unchanged |
| ArenaStop | 6 | Native | E / E | — | A | Writes ArenaFault; never returns |
| CalcAdr | 28 | ASM | E* / E | Y | unknown | Indexed-read/addend finding; writes `$E6/$E7` |
| Position | 5 | ASM | E / E | X Y | unknown | Incorrect N/Z fact; writes `$FF/$FE` |
| Relpos | 11 | ASM | E / E | X Y | unknown | A/NZ exit identity is ky; consistent with final ADC |
| Internal | 17 | ASM | E* / E | — | unknown | Indexed-read finding; A/NZ result `$A0` is valid |
| Ascii | 17 | ASM | E* / E | — | unknown | Same as Internal |
| _ImageAdvance | 23 | ASM | E / E | X Y | unknown | Writes `$E6/$E7,$A0/$A1`; path-sensitive A/NZ withheld |
| _LodChn | 6 | ASM | E / E | Y | unknown | No data-memory accesses; shifts A and sets X |
| _StrNam | 45 | Mixed ASM/native | U / U | — | unknown | Indexed writes; X is unchanged but summary loses that fact |
| Key | 34 | Native | U / U | X | A X | Read poisons writes; A exits equal to `$A0` |
| Next | 30 | Native | U / E | X | A X | Good read/write separation; A exits equal to `$A1` |
| Instr | 60 | Native | U / U | X | unknown | Array entry gate and read/write conflation |
| GetAnyKey | 8 | Native + barriers | U / U | — | unknown | Hardware poll `$D20F`; conservative barrier policy |
| Store | 10 | ASM | E / U | X | A X Y | Real indirect write; final N/Z from `LDY #0` is valid |
| Value | 11 | ASM | U / E | X | A X | Good read/write separation; A/NZ result `$A0` |
| Fnamecmp | 105 | Native | U / U | X | A X Y, `$A3`, C Z | Read/write and entry-flag precision gaps; uses `$A0-$A3` homes |
| TaggedFiles | 6 | Native | E / E | X Y | none | Reads active.taggedCount; A exits equal to `$A0` |
| Error | 0 | Empty declaration | E / E | — | none | Not an emitted callable body; no MIR callers |

For the five returning native functions, the ordinary analysis knows N/Z follows
the returned byte in A (the high byte for Next). The rewrite summary intentionally
withholds those flags because mutable MIR bodies lack a preservation contract
for incidental return flags. That is a deliberate safeguard, unlike Position's
incorrect machine fact. All published X/Y preservation claims in this census
agree with the emitted register writes.

Position's absent entry fact is also distinct from its bad exit fact: the bounded
machine entry decoder cannot resolve its symbolic Global operands to fixed bytes.
Other machine entries meet the decoder's restricted instruction/relocation
coverage limits. The residence selector currently rejects machine bodies and
ActionObservable callees anyway. Their entry gaps are not additional evidence
that native leaf placement is broken.

## Excluded wrappers and data, retained in the facts

All fifteen wrappers/dispatchers have unknown read and write summaries, no entry
certificate, and no X/Y preservation claim. GetD additionally publishes A and
N/Z equal to `$A0`; the N/Z fact is invalid as discussed above.

| Routine | Bytes | Why it is not a call-free leaf |
| --- | ---: | --- |
| Getchar | 23 | Stack-built RTS transfer to keyboard handler and back |
| Block | 71 | Calls CalcAdr |
| GetImage | 30 | Calls CalcAdr and _ImageAdvance |
| PutImage | 30 | Calls CalcAdr and _ImageAdvance |
| _Cio | 12 | Calls CIOV; can jump through ErrorHandler |
| GetD | 22 | Calls _LodChn and _Cio |
| PutD | 23 | Calls _LodChn; tail jumps to _Cio |
| Open | 28 | Calls _LodChn and _StrNam; tail jumps to _Cio |
| Close | 11 | Calls _LodChn; tail jumps to _Cio |
| Input | 32 | Calls _LodChn; tail jumps to _Cio |
| _BlockIO | 25 | Tail jumps to _Cio |
| Bget | 13 | Calls _LodChn; tail jumps to _BlockIO |
| Bput | 13 | Calls _LodChn; tail jumps to _BlockIO |
| Xio | 29 | Calls _LodChn and _StrNam; tail jumps to _Cio |
| MakeJmp | 22 | Indirect tail dispatch through `$A0/$A1` |

Menu data: drives (69 bytes), delcancel (23), density (52), yesabort (19),
lockunlock (21), yescancel (20). All have unknown memory summaries, no entry
certificate, and no register/return facts. Keeping these conservative is correct;
their disassembly cannot be used to infer executable effects.

## Diagnostic metadata limitation

The emitted map marks every routine as PROC with no return type/width:
`mir_routine_signature` currently hardcodes those fields. For example, TaggedFiles
has a byte result in its actual scalar signature but loses it in the map. Key,
Next and Fnamecmp also have meaningful direct parameter ABIs even though their
`scalar_signature` is None. The map is not a reliable source for callable type
facts. This is a reporting defect, separate from optimization correctness.

## Validation and reproduction

One diagnostic library test ran (3,449 other tests filtered out); it generated
and analysed TN in 19.41 seconds after a 3m15s test-binary build. It reproduced
the qualified image, then asserted byte/relocation equality on the layout replay.
The temporary test include was removed; `src/` remains unchanged.

The static scan covered 11 Exact read summaries and 12 Exact write summaries
across all candidates. All resolved, non-indexed data accesses were covered
(14 distinct read addresses and 22 distinct write addresses summed per routine).
There were no unresolved identities or contradictory X/Y preservation claims.
Manual inspection found the indexed-read and flag-provenance problems that this
simple footprint scan cannot detect. It is not a general proof of aliasing,
hardware behavior, computed transfers, or flags.

The independent Position control was compiled and executed in the existing VM
in two configurations, both demonstrating the defect. An exploratory
`enable_peepholes=false` arm failed pre-emission verification on an unlowered temp
and abstract Boolean branch; it did not provide a runtime control. No full TN
runtime campaign or broad compiler suites were run for this diagnostic-only task.

Generate facts (the expected-image option is optional if the earlier ignored
build artifacts are unavailable):

```sh
python3 tools/compare6502/tn/leaf_facts.py \
  --output build/tn-leaf-facts \
  --expect-image build/multiple-leaf-calls/tn-on/images/tn-ActionCart-3000.xex
python3 tools/compare6502/tn/audit_leaf_facts.py \
  build/tn-leaf-facts/facts.json \
  --output build/tn-leaf-facts/audit.json \
  --compact-facts build/tn-leaf-facts/facts.compact.json
```

The wrapper temporarily includes a diagnostic module inside known_callees to
read private analyses without widening production APIs. Do not concurrently
edit that file or build the same target. Its `finally` cleanup removes only its
own include. The compact projection stores shared MIR context once and retains
every per-routine fact. The generation receipt hashes the raw dump; audit.json
links that hash to the retained compact dump.

Reproduce the execution witness using the existing cached libraries (filenames
can differ after a build):

```sh
rustc --edition=2024 tools/compare6502/tn/leaf_facts_control.rs \
  --extern actionc=build/checked-arena/shared-target/debug/deps/libactionc-bf811c1ef8b2df31.rlib \
  --extern actionc_vm=build/checked-arena/shared-target/debug/deps/libactionc_vm-c3658201e399444a.rlib \
  -L dependency=build/checked-arena/shared-target/debug/deps \
  -o build/tn-leaf-facts/control
build/tn-leaf-facts/control \
  docs/benchmarks/6502-tn-leaf-facts/position-control.act build/tn-leaf-facts
```

The recommended order is: fix the proven N/Z soundness bug, repair indexed-read
footprint semantics, then decouple unknown reads from known writes and address
entry precision separately. This audit implements none of those compiler fixes.
