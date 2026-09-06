# Legacy integer arithmetic: observed behavior and compiler gaps

Status: historical audit complete; audited gaps repaired by the modern arithmetic
implementation, 2026-09-06. Tables below describe the audited baseline, not current output.
Audited actionc revision: `5777b0f83b92bec2b03457c3909ebafba5b90e5a`.
The audit changed no compiler, runtime, or test sources.

Implementation work is specified in the
[modern integer arithmetic plan](../MODERN_INTEGER_ARITHMETIC_IMPLEMENTATION_PLAN.md).
This note records observations, not a specification that actionc must emulate.

The [implementation record](../MODERN_INTEGER_ARITHMETIC_IMPLEMENTATION_PLAN.md#7-implementation-record-2026-09-06)
documents corrected folding, runtime kernels, operand captures, fault behavior,
target IR domains, and permanent conformance tests. Original ROM/SYSLIB bodies
and historical measurements remain unchanged; generated division/remainder
now uses compiler-owned helpers under both runtimes. Multiplication's INT
result type is deliberately retained, not confused with its raw output bits.

## Policy established after the audit

All actionc compilation profiles, including Compatibility, should implement
one correct, documented, target-independent arithmetic contract. Compatibility
is not permission to reproduce a compiler defect. Language-extension gates
and optimization choices can still differ between profiles.

The planned original-compiler-in-VM mode obtains historical behavior by
executing the original compiler. It is distinct from existing
`Runtime::ActionCart`: actionc must continue generating cartridge-dependent
programs without inheriting incorrect ROM arithmetic. Correct compiler-owned
helpers can be linked alongside calls to cartridge library services.

Original-compiler results are historical evidence, not the universal oracle
for modern conformance. Portable behavior must have independent expectations.
Changes to legitimate type/promotion policies require a deliberate language
decision; not every difference from another language is a cartridge bug.

## Evidence and limits

The main experiment compiled all 16 ordered BYTE/CHAR/INT/CARD operand-type
pairs in Compatibility, Optimized classic, and MIR6502, with ActionCart and
Standalone linking. Each of the 96 compiled programs ran in fresh VMs for
24 input pairs: **2,304 executions**, all reaching the completion marker.
This is an observation count, not 2,304 passing modern-conformance cases.

The public `compile_file` API was used. Each VM ran for at most 10,000 steps.
The host page was poisoned with `$CC`; input words were installed at
`$06E0/$06E2`, and `$06FF=$A5` marked completion. Loaded object segments
were checked not to overlap `$0600..$06FF`. BYTE/CHAR inputs consume the low
byte of each supplied word; they are not signed-byte cases.

The same fixture performed ordinary division/MOD and compound division/MOD.
Additional probes covered literals, typed CONSTs, zero divisors, multiplication
followed by division, and a guarded positive-input compound reproducer.
These temporary probes were not added to the permanent harness during the
read-only audit. The templates and input sets below preserve their essential
reproduction information without depending on temporary filesystem paths.

The isolated VM harness pins actionc-vm to
`7ec0cc454ebf43b088b7bcd11515533085ea1964`. A separate original-compiler run
used the sibling VM CLI and the repository ROM images. The CLI binary's build
revision was not independently established; the ROM hashes and observed
memory image identify the historical experiment:

| ROM | SHA-256 |
| --- | --- |
| `roms/action.rom` | `b4a3a399f4f1e8c20f4b1cbc3f6e2fbcef342c36d2c252f903938e93a502c166` |
| `roms/altirraos-xl.rom` | `9de5a313fe3946f04fe236a8d3ceacb471fbed4ec5fc5db009732e1169946ccf` |

68k and 65816 findings are source/IR audits, not execution results. Their
current backends are lowering canaries; public machine-code generation is
not yet implemented for those targets. No exhaustive 16-bit arithmetic proof
or broad multiplication/shift audit is claimed here.

## Findings

### ARITH-CONST-DOMAIN: constant and runtime operations disagree

The following values reproduced in all six actionc mode/runtime combinations.
INT results in this table are interpreted as signed values.

| Typed operation | actionc constant expression | Runtime inputs | Proposed portable result |
| --- | ---: | ---: | ---: |
| INT `-513 / 256` | 253 | -2 | -2 |
| INT `-513 MOD 256` | 255 | 2 | -1 |
| INT `7 / -3` | 0 | -2 | -2 |
| INT `7 MOD -3` | 7 | 2 | 1 |
| CARD `65535 / 2` | 32767 | 0 | 32767 |
| CARD `65535 MOD 2` | 1 | 0 | 1 |
| INT `-32768 / -1` | 0 | -32768 | -32768 under explicit wrapping policy |
| INT `-32768 MOD -1` | -32768 | 0 | 0 |

Explicit typed literal expressions and typed CONST declarations both exposed
unsigned folding of INT bits. For example:

```action
CONST INT quotient=INT(-513)/INT(256)
CONST INT remainder=INT(-513) MOD INT(256)
```

produced 253 and 255. These are unsigned operations on `$FDFF`, not signed
operations on -513.

Relevant evaluators include `evaluate_const_expr` in
[`semantic.rs`](../../src/semantic.rs), `const_u16_sem_expr` in
[`semantic/ir.rs`](../../src/semantic/ir.rs), classic `constant_u16` and
`constant_u16_with_defines` in [codegen](../../src/codegen.rs) and
[`codegen/data.rs`](../../src/codegen/data.rs), and `eval_binary` in the
[NIR optimizer](../../src/nir/optimizer.rs). Division/remainder operate on
unsigned `u16` values there instead of using the arithmetic domain.
The NIR lowerer's constant/address evaluator also needs inclusion in the
consolidation; it is not a separate authority for numeric semantics.

The original cartridge's literal expressions did not match actionc's folding:
`(-513)/256` returned -2, and `65535/2` returned 0. Thus the current
disagreement is not justified even as faithful original-compiler emulation.

### ARITH-CARD-SIGNED-HELPER: CARD uses signed runtime division

Classic selects `RuntimeHelperSlot::Div/Mod` by operator. MIR6502's
`helper_for_binary` likewise maps every division to `Div` and every
remainder to `Mod`, independent of signedness.

Cartridge linking binds these to `$A090/$A0DE`. Standalone linking selects
`DivI/RemI` from SYSLIB. Both normalize operands as signed words.
Consequently the upper half of CARD's range is misinterpreted:

- CARD 65535 / 2 yields zero, as signed -1 / 2.
- CARD 32768 / 2 yields bits `$C000` (49152 as CARD), not 16384.
- CARD 50000 / 40000 yields quotient 0 and MOD 15536, not 1 and 10000.

Sources: [classic arithmetic selection](../../src/codegen/arith.rs),
[MIR helper selection](../../src/mir6502/materialize/runtime.rs),
[cartridge binding](../../src/mir6502/runtime.rs), and
[standalone SYSLIB](../../corpora/action-runtime/extracted/SYSLIB.ACT).

### ARITH-LEGACY-REMAINDER: sign correction destroys the remainder

This is not merely a choice between truncating and Euclidean modulo.
`DivI` computes a quotient and a magnitude remainder, then performs quotient
sign correction. The negation routine `SS1` first saves its A/X operand into
`$86/$87`, overwriting the remainder. `RemI` calls `DivI` and returns
those bytes.

For opposite operand signs, this can return the quotient's magnitude:

| Runtime INT operands | Quotient | Legacy MOD | Truncating remainder |
| --- | ---: | ---: | ---: |
| -513, 256 | -2 | 2 | -1 |
| -7, 3 | -2 | 2 | -1 |
| 7, -3 | -2 | 2 | 1 |
| -1, 1 | -1 | 1 | 0 |
| -7, -3 | 2 | 1 | -1 |

The last row also shows that when quotient sign correction does not overwrite
the workspace, a positive magnitude remainder is still not the desired
signed remainder. Both sign decisions must be explicit in a replacement.

The original compiler/ROM and actionc's cartridge/standalone executions
reproduced the relevant cases. The source-level explanation is visible in
`SetSign`, `SS1`, `DivI`, and `RemI` in SYSLIB. Do not treat
`$86/$87` as a valid post-call remainder contract for the legacy divider.

### ARITH-ZERO-PANIC: zero handling depends on expression context

| Source context | Observed actionc behavior in all six combinations |
| --- | --- |
| `CONST q=1/0` | Diagnostic: division by zero in CONST expression |
| `CONST r=1 MOD 0` | Diagnostic: modulo by zero in CONST expression |
| Ordinary `q=1/0` | Compiles; runtime result bits `$FFFF` |
| Ordinary `q=1 MOD 0` | Compiles; runtime result 1 |
| Ordinary `q=1/BYTE(256)` | Compiles; runtime result bits `$FFFF` |
| Ordinary `q=(1/0)+1` | Compiler panic |
| `FOR i=0 TO 1 STEP 1/0 DO ... OD` | Compiler panic |

The panic occurs in `semantic/ir.rs::const_u16_sem_expr`:
`(right != 0).then_some(left / right)` evaluates the division eagerly.
The corresponding MOD expression has the same source defect; the audit
executed the division reproducers. A similar eager form exists in NIR's
constant/address evaluator and must be checked during repair.

Dynamic zero also has no reliable language contract. In the audited runtime,
INT -32768 / 0 returns 1, with MOD bits `$FFFF`, whereas 1 / 0 returns
`$FFFF`, with MOD 1. These are historical observations, not useful modern
sentinel values. Host panics and guest arithmetic faults are distinct issues.

### ARITH-CLASSIC-CAPTURED-RELOAD: a separate positive-input bug

This exact source was compiled with host-supplied inputs:

```action
INT a=$06E0,cq=$0604,cr=$0606
CARD b=$06E2
CARD q=$0600,r=$0602
BYTE done=$06FF
PROC Main()
  q=a/b
  r=a MOD b
  cq=a
  cq==/b
  cr=a
  cr==MOD b
  done=$A5
RETURN
```

Both runtimes give the following results:

| a, b | Ordinary MOD | Compatibility/MIR compound MOD | Optimized-classic compound MOD |
| --- | ---: | ---: | ---: |
| 513, 3 | 0 | 0 | 1 |
| 1000, 7 | 6 | 6 | 1 |
| 32767, 3 | 1 | 1 | 0 |
| 256, 3 | 1 | 1 | 0 |

All 24 focused executions completed; the inputs and surrounding poisoned
bytes remained unchanged. The ordinary and compound quotients agreed.

The optimized listing stages the first compound through a captured pointer.
During the later compound it uses `TXA` instead of loading the new operand's
high byte: X still contains the earlier quotient's high byte. This is stale
value reuse, not an arithmetic-policy difference. Investigate the lifetime
of stored memory facts and register aliases across captured-pointer reuse in
[`codegen/slot.rs`](../../src/codegen/slot.rs) and
[`codegen/state.rs`](../../src/codegen/state.rs). The audit does not claim
that a one-line fix or the complete invalidation boundary has been proven.

### ARITH-MIR-DOMAIN-LOSS: width does not identify the operation

NIR arithmetic retains a result type such as I16 or U16, but ordinary binary
lowering into [MIR6502](../../src/mir6502/lower.rs),
[MIR68k](../../src/mir68k/lower.rs), and
[MIR65816](../../src/mir65816/lower.rs) keeps only the operator and width.
Their binary forms do not retain the signed/unsigned arithmetic domain.
Comparison forms already carry signedness separately.

This is a target-selection contract gap, not evidence of an executed 68k or
65816 failure. Those backends must eventually select signed versus unsigned
operations without rediscovering source meaning.

### Related multiplication and helper-signature observations

The current `ScalarType::arithmetic_result` always returns INT for multiply.
That is a language policy requiring separate review, not a consequence of
which helper executes it. The NIR verifier currently enforces that policy.

With runtime BYTE inputs 255 and 255, `(a*b)/2` produced bits `$FF01`
(-255), while the literal equivalent produced 32512. An explicit outer CARD
cast still failed at runtime because division selected the signed helper.
This combines multiplication's current signed result type, unsigned constant
folding, and signed-only runtime division; changing just one can expose the
remaining defects.

MIR already supports a compiler-owned `MulByte` implementation with two
byte inputs and a word result. This is distinct from SYSLIB's identically
named internal `MultB` cross-term helper; names are not interchangeable ABI
contracts. The existing selection record separates operand and result width,
but has only one operand width shared by both inputs. Asymmetric helpers
such as `u16/u8 -> (u16 quotient, u8 remainder)` need per-input/per-result
contracts. See [MIR runtime materialization](../../src/mir6502/materialize/runtime.rs)
and the [compiler-owned multiply](../../src/mir6502/runtime.rs).

## Reproducing the observation matrix

Generalize the captured-reload fixture above by independently replacing the
left declaration's INT and the right declaration's CARD with each of
BYTE, CHAR, INT, CARD. Keep q/r as CARD output cells. Read cq/cr using the
left declaration's width. Compile once per type pair/mode/runtime, then run
fresh VMs with these unsigned input bit patterns:

```text
(0,1)         (1,0)         (65535,1)     (65535,2)
(32768,2)     (32768,65535) (65535,32768) (65023,256)
(65023,65280) (513,65280)   (65529,3)     (7,65533)
(65529,65533) (32767,256)   (65535,65535) (256,0)
(1,256)       (255,2)       (0,0)         (32768,0)
(100,7)       (128,129)     (255,255)     (50000,40000)
```

For permanent conformance tests, use independent typed mathematical oracles,
not these legacy outputs. Separate valid arithmetic, the chosen fault
contract, and original-compiler characterizations.

## Original-compiler reproduction

The historical run used this source:

```action
INT a,b
CARD u,v
CARD ARRAY out=$0600
BYTE done=$06FF
PROC Main()
 a=-513 b=256
 out(0)=a/b out(1)=a MOD b
 a=-7 b=3
 out(2)=a/b out(3)=a MOD b
 a=-7 b=-3
 out(4)=a/b out(5)=a MOD b
 u=65535 v=2
 out(6)=u/v out(7)=u MOD v
 u=50000 v=40000
 out(8)=u/v out(9)=u MOD v
 a=-32768 b=-1
 out(10)=a/b out(11)=a MOD b
 out(12)=(-513)/256 out(13)=(-513) MOD 256
 out(14)=65535/2 out(15)=65535 MOD 2
 a=1 b=0
 out(16)=a/b out(17)=a MOD b
 done=$A5
RETURN
```

Save that block to a temporary file. From the actionc repository root, with
the sibling VM executable available, substitute its absolute source path:

```sh
../actionc-vm/target/debug/actionc-vm run \
  --profile original-compiler \
  --cart roms/action.rom --os roms/altirraos-xl.rom \
  --hotpatch action-q-input --hotpatch action-headless-getkey \
  --host-file AUDIT.ACT:/absolute/path/to/audit.act \
  --monitor-key-at-pc '$A2E0' \
  --q-input-at-pc-after '$A2E0:$B2F5:C "H:AUDIT.ACT"\nR\n' \
  --stop-on-input-idle --max-steps 60000000 --history 8 \
  --dump-range-on-stop '$0600:$0623' \
  --dump-range-on-stop '$06FF:$06FF'
```

The host-file registration name is `AUDIT.ACT`; the Action! monitor opens
`H:AUDIT.ACT`. Including the device prefix in the registration argument
would be parsed incorrectly by this CLI.

The run stopped at input idle after compilation and execution, with:

```text
$0600: FE FF 02 00 FE FF 02 00 02 00 01 00 00 00 00 00
$0610: 00 00 B0 3C 00 80 00 00 FE FF 02 00 00 00 00 00
$0620: FF FF 01 00
$06FF: A5
```

## Disposition

The panic and captured-reload defect require independent general compiler
repairs. The other findings require a coordinated numeric contract, typed
constant evaluator, preserved target-facing arithmetic domain, and correct
runtime helpers. No mode should gain a bug-emulation implementation.

The existing compound-width tests and Oscar64 ports remain valuable but do
not cover these full domains. In particular, the compound VM fixture masks
its signed MOD inputs to nonnegative values. Its green status never established
correct negative MOD or full-range CARD division.
