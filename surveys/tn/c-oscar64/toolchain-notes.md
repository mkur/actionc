# Pinned Oscar64 integration observations

Compiler: `4da3147c1f53bfa543382a7e3febd113ceaec48f`. These observations belong to
this reference port; no external compiler or actionc source was patched.

- The ordinary C return of the shipped setjmp implementation is propagated as
  constant zero. The port discards that value and checks a volatile pending flag
  after the library restores the saved machine/software-stack state. Nested
  recovery tests validate the resulting control flow.
- A C arithmetic rendering of the inherited Relpos ADC carry chain caused an
  O3 `BinaryAddCarry` assertion. Relpos now retains the original machine algorithm
  with C argument loads. This preserves its carry from X addition into Y.
- Os combined Window's four parameter-to-zero-page assignments into a loop with
  an incorrect displacement. The generated listing had `LDX #3` at `$3008`,
  `LDA zp,X` at `$300A`, `STA zp,X`, `DEX`, and `BPL $3009` at `$300F`: the branch
  entered the LDX operand. Volatile stores alone still produced a broken loop.
  Three empty volatile assembly barriers keep scalar stores separate, insert no
  target instructions and preserve the four values/order. Both profiles execute
  correctly with this workaround. Build collection now rejects relative branches
  into addresses absent from the compiler-generated instruction listing.

TN's explicit Range scratch cells `$A0..$A3` remain fixed. Compiler address,
software-stack and frame registers move to `$D5..$D8`, `$D9/$DA`, `$DB/$DC`.
The temporary bank begins at `$A5`, its caller-clobbered portion is 16 bytes,
and the 32-byte saved bank is `$B5..$D4`. The build checks allocated IR temporary
lanes remain below `$D5`. Global automatic zero page is restricted to `$EA..$FD`;
fixed TN cells at `$E0/$E1`, `$E6..$E9`, `$FE/$FF` remain reserved.

The raw menu self references use pointer-typed relocation fields. Their two-byte
representation, payloads, keys, `$9A` tokens and next-entry offsets are checked
against the source format. This avoids relying on integer-cast pointer constants
in static aggregate initializers. Production code keeps normal inlining;
`TN_DIAGNOSTIC` explicitly selects callable controls for instrumentation.
