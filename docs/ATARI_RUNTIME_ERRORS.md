# Atari runtime errors

Integer division and MOD by a dynamic zero divisor use the existing `Error`
mechanism in every actionc compile profile. They do not create a separate
fatal-screen runtime. The target-independent fault remains `DivisionByZero`;
its numeric enum value is not an Atari error code.

Modern variant-value validation uses the same mechanism for `InvalidVariant`:
tag zero or an out-of-domain tag faults before the value or a matching arm is
exposed, including ELSE. Active inline nested variants are checked; pointers
and inactive alternatives are not traversed. This is actionc's new use of the
existing invalid-argument Error 100 convention, not historical cartridge ADT
behavior.

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

The standalone source is preserved separately:
[SYSLIB.ACT](../corpora/action-runtime/extracted/SYSLIB.ACT) declares
`PROC Error(BYTE err)[$6C$A$0$1113$8301]`. Its executable entry is
`JMP ($000A)`, i.e. DOSVEC; [SYS.DOC](../corpora/action-runtime/extracted/SYS.DOC)
explicitly describes that behavior. No message formatting is performed there.
The runtime loader's existing interface erratum makes it a raw three-byte
entry, preserving A/X/Y without an argument-copying prologue.

There is **no historical divide-by-zero-specific code to reproduce**. actionc
uses **100**, reusing the library's invalid runtime argument convention.
It must not forward the internal fault enum's value 1: cartridge code 1 means
a missing string quote.

## Delivery and linking

All compiler-owned signed, unsigned, narrow, and shared-divmod helpers prepare
A=100, X=0, Y=100 before calling Error. Supplying both A and Y supports
A-based custom handlers while satisfying the actual cartridge reporter.
Classic and MIR6502 invalid-variant paths reuse the same call convention and
non-return guard.

- Cartridge builds call the existing `$04CB` entry. The default handler prints
  `Error: 100` and enters the Action! monitor; existing dispatch behavior stays
  intact.
- Standalone builds link the original SYSLIB Error body and therefore transfer
  control through DOSVEC. They do **not** call an uninitialized `$04CB` entry,
  allocate a GR.0 screen, or promise that DOS will print the code.
  Error is recorded in the runtime-dependency map and existing standalone
  runtime-license warning, including for otherwise helper-only programs.
- If a handler returns, the helper clears decimal mode, restores A/Y=100,
  sets carry, and loops at `BCS self`. The failed operation never produces a
  normal result or executes subsequent source effects.

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
- `tools/vm-runtime-tests/tests/variants.rs` checks invalid tags, active nested
  validation, prior-state observation and returning handlers on both Atari
  backends, both runtimes, and raw/optimized NIR paths. NIR represents a variant
  fault as an opaque terminal call with unknown memory effects; its verifier
  rejects results, arguments, fallthrough or weakened effects.
