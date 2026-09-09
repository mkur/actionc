# Atari runtime errors

Compiler/runtime failures use the existing `Error` mechanism, with distinct
codes. No separate fatal-screen runtime is introduced. Target-independent
`RuntimeFault` reasons are mapped explicitly by `atari_runtime_error::code`;
their enum discriminants are not public Atari error numbers.

## Error codes

| Code | Reason | Reported by |
| --- | --- | --- |
| 100 | `InvalidArgument` | Generic invalid runtime argument; existing cartridge convention, not a catchall. |
| 101 | `DivisionByZero` | Division and MOD, all signed/unsigned widths and specialized/shared-divmod helpers. |
| 102 | `InvalidNumber` | Malformed decimal text in checked LONG conversions/input. |
| 103 | `NumberOutOfRange` | Well-formed decimal text outside the requested LONG integer range. |
| 104 | `InputTruncated` | Buffered LONG input record has no terminating EOL. |
| 105 | `InvalidVariantTag` | Zero or out-of-domain active variant tag. |
| 106 | `InvalidVariantOverlap` | Forbidden partial overlap in a checked variant-containing assignment. |

101–106 are actionc extensions, not historical cartridge definitions. The
preserved compiler error table allocates 0–28 and 61, and BREAK/CIO failures
start at 128. The library uses 100 for invalid Sound arguments; 101–106 are
not assigned in these preserved sources. Existing compiler/library/OS errors
are not renumbered or translated into these extension codes. This does not
reserve numbers against arbitrary user-defined handlers or third-party libraries.

Checked conversion syntax takes precedence over range: `4294967296x` reports
102, while `4294967296` reports 103. A minus sign is outside ValLC's accepted
grammar, so `-1` and `-0` report 102 there. Truncated input reports 104 before
parsing its prefix. Ordinary wrapping arithmetic does not acquire overflow
checks. Historical narrow and REAL conversion routines are unchanged.

Modern variant-value validation uses the same mechanism for `InvalidVariantTag`:
tag zero or an out-of-domain tag faults before the value or a matching arm is
exposed, including ELSE. Active inline nested variants are checked; pointers
and inactive alternatives are not traversed. Error 105 describes actionc ADT
validation, not historical cartridge behavior.

Checked variant-containing assignments use InvalidVariantOverlap/Error(106) when
source and destination partially overlap. Identical ranges are allowed, as are
disjoint ranges; uncertain pointer ranges are checked before copying. Source
address-expression effects precede the check, but a failing transfer writes no
destination bytes. This is not general pointer/lifetime validation; see the
[variant storage contract](VARIANT_STORAGE_CONTRACT.md).

The modern `SYS.ValLC`/`SYS.ValLI` and corresponding input routines use codes
102–104. They supply A/Y=code and X=0, then use the same defensive non-return
guard if the handler returns. See [32-bit SYS I/O](LONG_INTEGER_IO.md).

This code allocation does not change when checks are emitted. Moving overlap
guards to debug/checked builds is a separate agreed follow-up; they currently
remain enabled for uncertain checked transfers. It does not implicitly change
division-by-zero, conversion or variant-tag checking policy.

## Verified legacy contract

The preserved archive
`roms/source/action-3.6-source-0b8bcedb.tar.gz` contains these files under
`action-3.6-source-0b8bcedb/JAC/src/`:

| Source | Evidence |
| --- | --- |
| `main/MAIN.BNK.asm` | Public Error has three BYTE argument slots; Run installs the default SPLErr dispatch. |
| `lib/SPL.ERR.asm` | SPLErr invokes SysErr, then enters the monitor. |
| `main/MAIN.IO.asm` | `SysErr(,,errnum)` uses `TYA`, converts the number, and prints `Error: ` plus that number. The code is in Y, not A. |
| `lib/LIB.MSC.asm` | Invalid Sound voice uses `LDY #100; JSR Error`. |
| `ampl/AMPL.MTH.asm` | Original integer division has no explicit divisor-zero fault. |
| `compiler/COMPILER-DEF.asm` | Compiler errors 0–28 and 61; BREAK is 128. |
| `ACTION.DEF.asm`, `lib/LIB.IO.asm` | CIO error status has bit 7 set; existing OS codes are forwarded to Error. |

The standalone source is preserved separately:
[SYSLIB.ACT](../corpora/action-runtime/extracted/SYSLIB.ACT) declares
`PROC Error(BYTE err)[$6C$A$0$1113$8301]`. Its executable entry is
`JMP ($000A)`, i.e. DOSVEC; [SYS.DOC](../corpora/action-runtime/extracted/SYS.DOC)
explicitly describes that behavior. No message formatting is performed there.
The runtime loader's existing interface erratum makes it a raw three-byte
entry, preserving A/X/Y without an argument-copying prologue.

There is **no historical divide-by-zero-specific code to reproduce**. actionc
now uses **101**, replacing its earlier generic-100 fallback.
It must not forward the internal fault enum's value 1: cartridge code 1 means
a missing string quote.

## Delivery and linking

All compiler-owned division/remainder helpers (signed, unsigned, narrow, wide,
and shared-divmod) prepare
A=101, X=0, Y=101 before calling Error. Supplying both A and Y supports
A-based custom handlers while satisfying the actual cartridge reporter.
Classic and MIR6502 variant faults use the same convention with 105 or 106.
MIR's generic `Fault(RuntimeFault)` helper carries the reason to the Atari
adapter; all fault helpers have no arguments/results and opaque handler effects.

- Cartridge builds call the existing `$04CB` entry. The default handler prints
  `Error: <code>` and enters the Action! monitor; existing dispatch behavior stays
  intact.
- Standalone builds link the original SYSLIB Error body and therefore transfer
  control through DOSVEC. They do **not** call an uninitialized `$04CB` entry,
  allocate a GR.0 screen, or promise that DOS will print the code.
  Error is recorded in the runtime-dependency map and existing standalone
  runtime-license warning, including for otherwise helper-only programs.
- If a handler returns, the helper clears decimal mode, restores A/Y=code,
  sets carry, and loops at `BCS self`. The failed operation never produces a
  normal result or executes subsequent source effects.
  LONG conversion wrappers preserve their dynamic code on the hardware stack
  across the handler, rather than depending on handler-preserved registers/RAM.

This intentionally delegates screen policy to Error. Cartridge SysErr enables
the display/restores its background, but does not itself allocate a new
`Graphics(0)` screen. Reliable fatal output from an arbitrary custom graphics
state would require a separate, explicit runtime policy change.

The public `SYS.Error` API is unchanged. Its raw arguments are A/X/Y;
`Error(code,0,code)` supplies a deterministic code for both A-based handlers and
the cartridge. A one-argument call alone does not establish Y.

## Optimizer contract and tests

Potentially faulting arithmetic is an observation boundary: the handler can
inspect prior fixed or escaped storage. NIR preserves/synchronizes those writes;
MIR models arbitrary handler memory/OS effects rather than just successful
arithmetic scratch. A proven nonzero divisor needs no NIR fault barrier.

- `tools/vm-runtime-tests/tests/action_error.rs` boots the real ROM and passes
  distinct A/X/Y values, proving that the original formatter reports Y.
- `tools/vm-runtime-tests/tests/modern_arithmetic.rs` exercises both link modes
  and all three compile profiles, including the unmodified cartridge handler,
  a returning handler, prior-state observation, and no post-fault effects.
- `tests/modern_integer_arithmetic.rs` checks that optimization retains the
  prior fixed-local store even without an address-taking expression.
- Shared helper tests verify all ten signatures' Error relocation and defensive
  non-return guard, alongside existing nonzero arithmetic/scratch oracles.
  Wide-helper tests cover the four 32-bit division/remainder kernels as well.
- `tools/vm-runtime-tests/tests/long_integer_io.rs` checks codes 102–104 for both
  signednesses and runtimes, syntax/range precedence, preserved result storage,
  and a returning handler that clobbers A/X/Y and sets decimal mode.
- `tools/vm-runtime-tests/tests/variants.rs` checks invalid tags, active nested
  validation, partial-overlap rejection, prior-state observation and returning handlers on both Atari
  backends, both runtimes, and raw/optimized NIR paths. NIR represents a variant
  fault as an opaque terminal call with unknown memory effects; its verifier
  rejects results, arguments, fallthrough or weakened effects.
