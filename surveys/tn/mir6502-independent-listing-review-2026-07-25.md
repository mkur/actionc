# Independent TN MIR6502 listing review

Date: 2026-07-25

Status: artifact-pinned listing review. Recheck every candidate against the
current compiler head before implementing it.

Artifact: `TN-mir6502.lst`

- SHA-256: `cc1baff8f33972dbd3e5b298890e8ac210462e0347a51f53c97bc0d6c14d24cc`
- Text size: 134,359 bytes, 5,178 lines
- Emitted primary range: `$2C00-$5344`, 10,053 bytes
- Scope: final 6502 listing only; no pre-materialized or materialized MIR was
  available for this review

This artifact is not the listing frozen by the
[2026-07-24 final-listing audit](mir6502-final-listing-audit-2026-07-24.md):
that file is 139,097 bytes with SHA-256
`5f447d3abd62ab9b07ad6149d889f03175458532a5b922e3f54428e34707ffd4`
and has a 10,443-byte primary segment. The observations below therefore apply
to the exact artifact hash above and are not a claim about an older or newer
named revision.

> [!IMPORTANT]
> The first listing-only interpretation of `r_Par` as evidence for a caller-side
> `$A0-$A2` ABI was wrong. The canonical Action ABI passes argument bytes 0, 1,
> and 2 in A, X, and Y, and later bytes in `$A3+`. `r_Par` is a **callee-side
> formal-parameter capture helper**. The actionable opportunity is selective
> formal-home materialization or specialization, not replacing the canonical
> ABI. See the implemented
> [caller-shadow ABI correction plan](mir6502-public-abi-shadow-correction-plan.md).

## Overall assessment

The generated code is already aware of several important 6502 idioms. It uses
tail `JMP`s in many wrappers, reasonable 16-bit increment sequences, and
indexed IOCB accesses. The remaining inefficiency mostly comes from losing
useful facts before final instruction selection:

- a fixed symbol-plus-index expression becomes a constructed pointer;
- a pointer or byte value is materialized in RAM and immediately reloaded;
- calls are treated as clobbering more state than their known bodies actually
  modify;
- byte widths and narrow ranges are widened into generic 16-bit arithmetic;
- the final machine pass leaves several safe local cleanups behind.

The largest likely gains are therefore not from isolated mnemonic peepholes.
They are from preserving address shape, call effects, value location, width,
and range information through MIR6502 lowering, followed by a strong
post-home machine cleanup pass.

## Static measurements

The seven embedded data blocks listed below are marked as procedures and can
be misread as code. After reclassifying them, the listing contains:

| Item | Result |
| --- | ---: |
| Real 6502 instruction rows | 4,024 |
| Real instruction bytes | 9,007 |
| Non-instruction payload | 1,046 bytes |
| Primary segment payload | 10,053 bytes |
| Routine-local/formal/spill region | 196 bytes, `$2E53-$2F16` |
| Direct instruction references into that region | 473 |
| Procedures beginning with the generic `r_Par` capture | 11 |
| `STA $A0` immediately followed by `RTS` | 25 |
| Explicit `CMP #$00` | 12 |
| Exact `JSR target; RTS` tail-call shapes | 4 |
| `JMP` whose target is an `RTS` | 6 |

The three largest procedures are:

| Procedure | Instruction bytes |
| --- | ---: |
| `SetWin` | 1,030 |
| `Handle` | 820 |
| `Copy` | 665 |

Together they account for 2,515 bytes, about 28% of all real instruction
bytes. Work that applies inside these routines will dominate small global
peepholes.

## Preserve code/data classification

The following ranges are data even though they are emitted as procedure
ranges:

| Name | Range | Bytes |
| --- | --- | ---: |
| `drives` | `$44DD-$4521` | 69 |
| `delcancel` | `$4522-$4538` | 23 |
| `density` | `$4539-$456C` | 52 |
| `yesabort` | `$456D-$457F` | 19 |
| `lockunlock` | `$4580-$4594` | 21 |
| `yescancel` | `$4595-$45A8` | 20 |
| `Jmp` | `$4EAA-$4EBB` | 18 |
| Total | | 222 |

Their bytes decode into plausible and implausible mnemonics by accident,
including `TXS`, `BRK`, and illegal opcodes. This accounts for 97 apparent
instruction rows and 181 apparent instruction bytes; the remaining bytes are
already shown as `.BYTE`.

The listing representation should retain a semantic code/data class and emit
these ranges as `DATA`, `.BYTE`, or `.WORD`. The listing-quality parser should
also trust that classification rather than infer code from whether a byte
sequence happens to decode.

## Correctness observations

### `MovePage` copies 256 bytes for a zero length

At `$305B`, the routine loads the length from `$A4`, decrements it, and then
uses `$FF` as the loop terminator:

```asm
    LDY $A4
    DEY
loop:
    LDA ($A2),Y
    STA ($A0),Y
    DEY
    CPY #$FF
    BNE loop
```

When the incoming length is zero, Y becomes `$FF` before the first load and
the routine copies all 256 offsets. `Strcat` passes a source string length
directly, so an empty source string is a possible trigger unless a source-level
precondition rules it out.

A defensive implementation is:

```asm
    LDY $A4
    BEQ done
    DEY
loop:
    LDA ($A2),Y
    STA ($A0),Y
    DEY
    CPY #$FF
    BNE loop
done:
    RTS
```

This should be covered by a runtime test for a zero-byte move and by an empty
string concatenation test.

### `PrintB` suppresses an internal zero

`PrintB` prints the hundreds digit only when nonzero, then independently
suppresses a zero tens digit. A value such as 105 therefore appears to produce
`15`, not `105`.

This looks like a source or runtime-library algorithm issue rather than a
compiler miscompile, but it is a useful regression test because the routine
also exercises byte division, remainder handling, return-value propagation,
and character output.

## Optimization opportunities

### 1. Specialize or avoid callee-side `r_Par` materialization

Eleven procedures begin with:

```asm
    JSR r_Par
    .BYTE destination-low, destination-high, byte-count-minus-one
```

They are `Window`, `Range`, `InputLine`, `Ord`, `FindItem`, `MoveMenuBar`,
`PopUp`, `Strcpy`, `Strcat`, `Instr`, and `Fnamecmp`.

`r_Par` saves the canonical incoming A/X/Y values in `$A0-$A2`, advances the
return address over the inline descriptor, and copies the flattened argument
bytes into a private formal frame. Including the caller's `JSR`, the approximate
cost is `15n + 79` cycles for `n` copied bytes:

| Captured bytes | Approximate cost |
| ---: | ---: |
| 3 | 124 cycles |
| 4 | 139 cycles |
| 5 | 154 cycles |

This is not a reason to change the Action calling convention. It is a reason
to materialize formal storage lazily:

- use direct `STA`/`STX`/`STY` capture for small frames when that is cheaper;
- omit a formal cell when it is write-only and its address is unobservable;
- keep incoming values as MIR operands until an address use, join, call
  boundary, or machine effect actually requires backing storage;
- retain the generic helper for address-taken frames and cases whose
  observability proof is incomplete.

The repository already distinguishes the incoming ABI from callee capture.
Any implementation here should reuse that existing proof boundary.

### 2. Propagate known callee exit state instead of changing the return ABI

There are 25 return paths ending in:

```asm
    STA $A0
    RTS
```

The `$A0` store may be required by the Action result convention or by an
observable routine boundary, so it should not be deleted globally. The
listing nevertheless contains callers that reload `$A0` immediately even
though A still holds the same result.

For example, around `$4281`:

```asm
    JSR Internal
    LDY $E0
    LDA $A0
    STA ($AC),Y
```

A precise summary for `Internal` can state that the result leaves A and Z
valid. The caller can then store A directly while preserving any required
callee-side `$A0` write. Similar shapes occur near `$3941` and `$4387`.

The same contract removes many of the 12 explicit `CMP #$00` instructions
after boolean-returning calls. The safe rule is not “all calls preserve
flags”; it is “a known direct callee's modeled exit state establishes the
result and flags on every return path.”

### 3. Select absolute-indexed addressing before constructing pointers

The compiler frequently lowers a fixed array access into:

```asm
    LDA base-low
    STA $AC
    LDA base-high
    ADC #$00
    STA $AD
    LDA ($AC),Y
```

That is appropriate for a dynamic pointer, but not for a statically located
object.

At `SetWin $4107`, a pointer to `dirnames` at `$2C48` is constructed before
two adjacent stores. The sequence can be reduced to the direct indexed form:

```asm
    ASL A
    TAY
    LDA $2EE1
    STA $2C48,Y
    LDA $2EE2
    STA $2C49,Y
```

Related opportunities occur in:

- `Handle`, when reading `dirnames`;
- `Copy`, when indexing the word table near `$5E05`;
- the pointer table rooted at `$2C44`;
- source-string and fixed local-array accesses.

MIR6502 should preserve a distinction among:

```text
load symbol[index]
load *(pointer + index)
address_of symbol[index]
```

Only the latter two require an effective pointer. Once all three have become a
generic materialized address, a peephole must reconstruct information the
selector already had.

### 4. Hoist stable pointers and lengths out of loops

The listing repeatedly copies a two-byte RAM pointer into a zero-page
indirect pair. Many copies occur inside loops where no intervening operation
changes either the pointer or the length.

Examples:

- `Print $3133` reloads the string pointer and length for every character.
  `Putchar` does not need to invalidate the source pointer.
- `Instr $3A00` has no calls in its scan loop, yet reloads both pointer and
  length each iteration.
- `Convert $3E57` reads the same source byte twice, reloads a call result from
  `$A0`, and rebuilds pointers that `Ascii` does not clobber.
- `Fnamecmp $3A96` is a 174-byte two-pointer comparison with no calls in its
  hot loop, but repeatedly moves both pointers between RAM homes and zero-page
  pairs.

A routine-local value-location pass should keep stable pointers in `$AC/$AD`
or `$AE/$AF` for their live intervals and retain loop-carried indices in Y
where possible. Because Atari memory may be memory-mapped I/O, redundant-load
elimination must still distinguish ordinary RAM from volatile or unknown
memory.

### 5. Use precise call and scratch-pair effect summaries

The code often behaves as if a direct call destroys every register, flag, and
private pointer pair. Each routine should have a machine-level summary for:

- A, X, and Y clobbers;
- N, Z, C, and V exit validity;
- fixed zero-page pair reads and writes;
- arbitrary-memory reads or writes;
- volatile I/O or opaque machine effects;
- purity or read-only behavior where provable.

Examples:

`Putchar $3120` currently spills its byte across `CalcAdr`:

```asm
    STA $2E60
    JSR CalcAdr
    LDA $2E60
    JSR Internal
```

`CalcAdr` does not touch Y, so the value can be held with `TAY`/`TYA`, avoiding
the absolute spill.

`Strcat` reloads source and destination pointers after `MovePage` even though
that helper does not modify the relevant pairs.

Several `SetWin` loops rebuild the same pointer around calls whose known bodies
do not clobber it. These are better solved by transitive callee summaries than
by isolated textual peepholes.

### 6. Schedule copies for a one-accumulator machine

The scheduler sometimes loads both bytes before storing either and then uses
the hardware stack or X as temporary storage. In `Handle`, a two-byte copy is
emitted in the shape:

```asm
    LDY #0
    LDA ($AC),Y
    PHA
    INY
    LDA ($AC),Y
    TAX
    PLA
    DEY
    STA ($AA),Y
    TXA
    INY
    STA ($AA),Y
```

When alias analysis proves that the source and destination ordering is safe,
the 6502-friendly schedule is:

```asm
    LDY #0
    LDA ($AC),Y
    STA ($AA),Y
    INY
    LDA ($AC),Y
    STA ($AA),Y
```

The same family appears again later in `Handle`. The scheduler should prefer
load-byte/store-byte ordering unless overlap or volatile semantics require all
loads to precede the stores.

### 7. Preserve byte widths and narrow ranges

Several expressions with byte-sized or tightly bounded inputs reach generic
signed 16-bit helpers.

#### Decimal byte printing

`PrintB` performs four helper calls:

1. divide by 100;
2. compute `% 100` with another full division;
3. divide the remainder by 10;
4. compute `% 10` with another full division.

`r_Div` already leaves a remainder in its scratch state, so the generic
implementation should need at most two divisions. For an unsigned byte, a
dedicated subtract-100/subtract-10 conversion is smaller and faster still.

#### Multiplication by 20

`DrawWinFrame` and `SwapWin` use the generic signed multiplier for `x * 20`.
Where range facts prove a small nonnegative byte, use either a table or:

```text
x * 20 = x * 16 + x * 4
```

For `winnum` constrained to 0 or 1, a conditional or two-entry table is
simpler than any multiplier.

#### Unnecessary carry-preservation machinery

`Handle` proves `nestLevel < 4` and then doubles it, yet the generated address
calculation still saves shift carry with `PHP`, reconstructs it with `ROL`,
and restores flags with `PLP`. The shift cannot overflow under the proven
range.

`Sort` contains a related case where an index is checked below `$80` before
doubling. Range facts should survive until address selection so these
general-purpose carry paths can be omitted.

#### Canonical loop bounds

Several loops compute `count - 1` and then compare with the index. For example,
`Copy` can test `k >= files` directly instead of materializing
`files - 1`. The direct form is shorter and avoids underflow when `files == 0`.

### 8. Merge duplicated `SetWin` tails and reuse addresses

`SetWin` is the largest procedure and contains several local CSE and tail
merging opportunities:

- The two arms around `$4214/$4234` choose offset 0 or 1, call the same
  conversion routine, and store to the same final offset. Select the input
  first and share the call/store tail.
- Around `$4317`, the same base pointer is established separately for stores
  at offsets 10 and 14. Establish it once.
- Around `$4359`, `s[i]` is loaded for termination checks, the pointer is
  rebuilt, and `s[i]` is loaded again for conversion. Keep the first byte in A
  and the stable pointer in its pair.

These should be implemented through local value numbering and effect-aware
address retention, not as `SetWin`-specific patterns.

### 9. Reuse a proven read-only call result

`PopUp` calls `FindItem(menu, key)` to test for null and then calls
`FindItem(menu, key)` again on the non-null path with no apparent intervening
menu mutation.

If `FindItem` is summarized as read-only and the menu object is unchanged, the
first pointer result can be retained and reused. This requires multi-byte
result facts, dominance, and call-memory effects; it should not be a general
“identical calls are pure” assumption.

### 10. Strengthen post-home machine cleanup

Several low-risk final forms remain.

#### Tail calls

The listing contains four `JSR target; RTS` pairs:

- `Delete`
- `Attrib`
- `InitPanels`
- `NavError`

Each can become `JMP target`, saving one byte and nine cycles per execution.
These shapes also exist in the classic backend, so they are general code
quality work rather than an explanation of a MIR6502/classic size gap.

#### Jumps to return blocks

Six jumps target a block containing only `RTS`:

- `Sort`
- `Tag`
- `Format`
- two paths in `SwapScr`
- `Quit`

Replace each jump with `RTS` when the target has no additional edge semantics.

#### Register-store folding

These exact shapes occur in `Free`, `Push`, `Key`, and `Next`:

```asm
    TXA
    STA zp
```

Use `STX zp`. Do not generalize this to unsupported NMOS 6502 addressing modes.

#### Dead stores and dead loads

Examples visible in this artifact include:

- `SwapWin $444A`: a private store of the new window number whose value is
  passed directly in A;
- `PrintB $44A0` and `$44BE`: character stores before `Putchar` while A already
  holds the character;
- `Attrib`: a command stored in a private byte while the call receives A
  directly;
- `MakeJmp`: an incoming byte stored in a cell that is never read;
- loads at `$4E98` and `$4FD2` that are overwritten before use.

A CFG-aware machine DCE pass must preserve opaque machine-block and external
entry edges, but ordinary unreachable or overwritten operations should not
survive final emission.

#### Local control-flow simplification

Other small examples include:

- branch directly to the reachable loop target in `Fnamecmp` instead of
  branching to a `JMP`;
- share the final store in `GoTo`;
- remove duplicate `LDY #0` and repeated loads in both branches of `Range`.

### 11. Improve zero-page allocation without treating all locals as spills

The `$2E53-$2F16` region contains 196 bytes of private formal, local, and spill
storage and is referenced 473 times. Not all of this storage is removable:
Action locals may be addressable, aliased, or visible to machine code.

The useful allocator work is narrower:

- coalesce exact aliases before assigning homes;
- split live ranges around calls rather than spilling a value for the entire
  routine;
- rematerialize constants and fixed addresses;
- color only non-address-observable spill slots by liveness;
- prioritize hot loop-carried pointers and bytes for zero page;
- keep one-use and short-lived values in A, X, or Y;
- do not use broad frame pooling as a substitute for removing load/store
  traffic.

`Copy`, `SetWin`, `Handle`, and `InputLine` are useful stress cases for this
work.

### 12. Preserve old-value semantics through scheduling

`Alloc $306E` saves the requested size, updates the allocation pointer, and
then subtracts the same size from the new pointer to recover the old pointer.

The old pointer is available before the update. MIR should retain:

```text
old = pointer
pointer += size
return old
```

long enough for the scheduler to save or return the old value directly. This
removes the add-then-subtract reconstruction and its temporary pair. The same
principle applies to post-increment expressions elsewhere in the backend.

## Recommended implementation order

1. Pin this artifact in any follow-up measurement and fix code/data
   classification in the listing and quality parser.
2. Add correctness tests for zero-length `MovePage` and internal-zero `PrintB`.
3. Run post-home machine DCE, tail-call conversion, jump-to-return folding,
   register-store folding, and local CFG cleanup.
4. Specialize or lazily materialize callee formal frames without changing the
   canonical Action ABI.
5. Expand known-callee exit, scratch-pair, register, flag, and memory-effect
   summaries.
6. Preserve fixed symbol-plus-index address forms and retain stable pointers
   through loops and safe calls.
7. Carry byte width and range facts into arithmetic and address selection.
8. Revisit allocation only for concrete remaining consumers, using
   coalescing, rematerialization, live-range splitting, and zero-page
   prioritization.
9. Remeasure the whole XEX after every slice. Routine-local savings can be
   offset or amplified by data placement and layout changes.

The best near-term regression procedures are `Fnamecmp` for pointer retention,
`Handle` for two-address scheduling, `SetWin` for address CSE and high pressure,
`PrintB` for byte arithmetic, and `Alloc` for old-value scheduling.
