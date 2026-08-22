# Action 2027 Runtime Interface and Standalone Linking

Design status: implemented for the experimental standalone kernel and its
initial internal `SYS` surface. This note defines how one resolved Action
program can use either the Action! cartridge implementation or selectively
included OSS runtime source. The module syntax below describes compiler-owned
infrastructure; users select the runtime on ordinary single-file source without
adopting modules.

## Goals

- Keep `SYS` source names and signatures identical under every runtime.
- Let users choose compact cartridge-backed output or cartridge-independent
  standalone output.
- Embed the OSS runtime sources in the single compiler executable.
- Include only runtime routines reachable from the program or explicitly
  required by target lowering.
- Preserve original Action! helper overrides without silently weakening the
  standalone guarantee.
- Keep runtime selection independent of classic versus MIR6502 code generation
  and independent of optimization profile.

## Runtime Choice

The command-line option uses a separate argument:

```sh
actionc --backend mir6502 --runtime standalone program.act
actionc --backend mir6502 --runtime cart program.act
```

The same choices are available through the compiler API:

```text
Runtime::Standalone
Runtime::ActionCart
```

The two modes are:

| Runtime | Compiler helpers | `SYS` implementations | External requirement |
| --- | --- | --- | --- |
| `standalone` | select and emit embedded OSS routines | select and emit reachable source implementations | Atari OS as required by the chosen routines; no Action! cartridge |
| `cart` | call resident helper entry points | call resident library entry points | Action! cartridge with the expected ABI |

The initial implementation retains the existing cartridge default to avoid a
silent compatibility change. Switching the general default to standalone is a
deliberate later release decision after runtime coverage is sufficient.
`--runtime cart` remains permanently available for exact comparison and small
cartridge-backed programs.

No `auto` or hybrid mode is introduced initially. Identical compiler arguments
must not acquire different external requirements from the host environment.
Once a program requires the cartridge, selectively embedding alternative
copies of routines usually adds complexity without removing that dependency.

## Backend Orthogonality

`--backend` answers how user and source-runtime code is lowered. `--runtime`
answers where support implementations come from. They are separate axes:

```text
classic + cart
classic + standalone
mir6502 + cart
mir6502 + standalone
```

MIR6502 was the first implementation target for selective runtime linking, but
`--runtime standalone` does not semantically imply or silently select
`--backend mir6502`. Classic and MIR6502 now both support cart and standalone
runtime selection.

This separation permits MIR6502-optimized application code to use compact
cartridge routines and permits classic code generation to become standalone
without changing source semantics.

## Target-Neutral `SYS` Interface

`SYS` is a real embedded Action source module parsed by the ordinary frontend.
Its public declarations own names, types, signatures, and source-visible
effects. They do not expose one runtime's physical address as the public API.

Conceptually:

```action
MODULE SYS
  PUBLIC EXTERNAL PROC PrintE(CHAR ARRAY text)
  PUBLIC EXTERNAL PROC Graphics(BYTE mode)
ENDMODULE
```

`PUBLIC EXTERNAL PROC/FUNC` and compiler-owned `SET` binding units are the
accepted syntax defined in
[`EXTERNAL_RUNTIME_BINDINGS.md`](./EXTERNAL_RUNTIME_BINDINGS.md). The central
design invariants are:

- the public `SYS.PrintE` declaration exists once;
- the cart binding maps it to `$A46C`;
- the standalone binding maps it to the resolved OSS runtime routine;
- both bindings carry the same callable ABI expected by the interface;
- a missing or ABI-incompatible binding is a compile-time error;
- binding facts do not live in a duplicated Rust string table.

Bindings themselves may be represented by embedded Action interface/binding
source interpreted by the compiler. Separate cart and standalone bindings must
reference the one interface symbol rather than copy its declaration.

A fixed address is an implementation property. `@SYS.PrintE`, a routine
pointer, a static initializer, or a qualified `SET` value resolves to the
selected implementation address after runtime binding. Code cannot depend on
the cart address while claiming to be standalone.

### Compatibility prelude

Traditional unqualified resident names remain available through an implicit
compatibility prelude. When a routine has migrated, its unqualified spelling
and `SYS` member are aliases of one symbol and select either the cart or
standalone implementation. Standalone compilation still rejects an unmigrated
resident call rather than retaining its cartridge address. During the
standalone rollout, ordinary source continues to use the traditional
unqualified spelling; the `SYS` identity remains compiler-owned infrastructure.

The full interface is available during semantic analysis, but SemIR retains
only external routines referenced by executable code or static data. The
implicit prelude therefore does not add unused routines to NIR, maps, or the
generated program.

Additional resident names can migrate to aliases of the same `SYS` identities
as their standalone implementations are added. The prelude must not become a
second implementation catalog.

## Logical Runtime Requirements

Runtime dependency discovery occurs before final address emission.

SemIR resolves public library routines to stable routine identities. NIR keeps
those identities without physical target addresses. MIR6502 records logical
helper requirements such as:

```text
MirRuntimeHelper::Lsh
MirRuntimeHelper::Rsh
MirRuntimeHelper::Mul
MirRuntimeHelper::Div
MirRuntimeHelper::Mod
MirRuntimeHelper::SArgs
```

The current MIR6502 implementation maps these directly to cartridge addresses.
That mapping must move to runtime resolution. MIR helper declarations retain
their ABI and conservative effects but remain physically unresolved until the
selected runtime provider supplies a target.

Classic must converge on the same logical requirement model rather than infer
runtime choice from storage layout or fixed helper slots.

## `SArgs` and `r_Par`

Action!'s runtime source calls the parameter-copy helper `SArgs`; cartridge
maps and existing generated listings commonly call it `r_Par`. A call whose
complete argument frame is larger than the three bytes carried directly in A,
X, and Y requires this helper under the Action ABI.

Under `--runtime cart`, `SArgs` resolves to the resident entry point currently
known as `$A0F5`. Under `--runtime standalone`, it resolves to the `SArgs`
procedure parsed from embedded `SYSLIB.ACT` and is emitted only if required.

A source program should not normally need this compatibility pattern:

```action
PROC r_Par=*()
  ; custom local helper
RETURN

SET $4EE=r_Par
```

It nevertheless remains supported under the precedence rules below.

## Explicit Helper Override Precedence

Original Action! uses `SET` at `$04E4` through `$04EE` to redirect helper
slots. A recognized explicit override has higher precedence than the selected
runtime provider.

Resolution order is:

1. a valid source-level helper override;
2. the selected `cart` or `standalone` runtime binding.

The target of the override determines whether the standalone guarantee remains
valid:

- `SET $4EE=r_Par`, where `r_Par` is a resolved local routine emitted with the
  program, is valid under either runtime. It suppresses automatic inclusion of
  `SYSLIB.ACT::SArgs`.
- An override to another relocatable routine included in the program follows
  the same rule; it need not be `PUBLIC` merely because it is local to the root
  program.
- An override to an absolute external address is valid under `--runtime cart`.
- An override to an absolute external address under `--runtime standalone` is
  an error because the compiler could no longer guarantee cartridge
  independence.

Standalone never emits its embedded helper in addition to a valid local
override. Cart mode never silently ignores an original source override.

## OSS Runtime Sources

The authoritative source is already present under
`corpora/action-runtime/extracted`:

- `SYSLIB.ACT` defines `LShift`, `RShift`, `MultI`, `DivI`, `RemI`, and
  `SArgs`, with the original helper-slot assignments;
- `SYSBLK.ACT`, `SYSIO.ACT`, `SYSGR.ACT`, `SYSMISC.ACT`, and `SYSSTR.ACT`
  contain the split standard library;
- `SYS.ACT` and `SYSALL.ACT` document the combined surface and dependencies.

The extracted sources retain historical headers. The later OSS release
licenses Action! under GPL-3.0-or-later, as recorded in
[`../../roms/ACTION-ROM-NOTICE.md`](../../roms/ACTION-ROM-NOTICE.md) and
[`../../corpora/action-runtime/README.md`](../../corpora/action-runtime/README.md).

These files are direct embedded-VFS inputs, not merely behavioral references
and not copies maintained under a second runtime directory. If a source must be
adapted, preserve its provenance, mark the modified version, and retain the
GPL-3.0-or-later terms.

Generated programs containing selected GPL runtime routines remain subject to
the applicable runtime license. Maps, listings, and release documentation must
identify the included source routines and preserve access to corresponding
source.

## Selective Inclusion

Importing `SYS` exposes an interface but includes no code by itself. After
semantic resolution and target lowering, the compiler computes a runtime
dependency closure from:

1. referenced `SYS` routine identities;
2. address-taken `SYS` routines and conservative indirect-call roots;
3. logical helpers required by MIR6502 or classic lowering;
4. explicit source overrides selected in place of default helpers;
5. dependencies introduced by already selected runtime routines.

Runtime call cycles are collapsed into small strongly connected groups. A
selected member emits the complete inseparable group; unrelated groups remain
absent.

Each selected routine or group carries:

- its resolved stable identity;
- relocatable code and data;
- required static and zero-page storage;
- callable ABI and conservative effects;
- relocations to user, OS, and runtime symbols;
- its source origin and GPL provenance;
- dependencies on other runtime routines or logical helpers.

Only the closure is laid out and emitted. Historical top-level `SET` statements
inside `SYSLIB.ACT` document original bindings but do not root every helper in
standalone output.

Classic standalone emission places the selected runtime closure after the
application's source-controlled layout. This preserves programs that set the
compatibility code pointer explicitly: runtime selection must not occupy that
address range before the application has applied its own `SET $E` or
`SET $491` directives. Helper binding directives remain compiler-visible
independently of module emission order.

Runtime-specific static or zero-page storage is reserved only when a selected
routine requires it. ABI slots used directly by generated program calls remain
part of the target ABI independently of runtime selection.

## Failure Policy

Runtime independence must fail closed:

- a missing standalone implementation is a compile-time error, never a silent
  fallback to a cartridge address;
- a missing cart binding is a compile-time error;
- an ABI mismatch between `SYS` and a selected implementation is an error;
- an absolute external helper override is rejected in standalone mode;
- unresolved runtime dependencies are rejected before final emission;
- a resident routine without a standalone binding is rejected instead of
  retaining its cartridge entry point.

Diagnostics name the logical routine, selected runtime, requesting source call
or lowering operation, and available alternatives when useful.

## Runner Integration

`actionc-run` must propagate runtime selection into compilation and emulator
startup as one coherent choice:

```text
--runtime standalone -> compile for standalone; launch without cart
--runtime cart       -> compile for cart; mount the bundled cart
--no-cart            -> convenience form of --runtime standalone
--cart PATH          -> imply --runtime cart; mount PATH
```

The runner accepts explicit, consistent combinations such as `--runtime cart
--cart PATH`, but contradictory combinations are errors rather than precedence
puzzles. It resolves the runtime and cartridge choices together and checks the
invariant again before compiling the ATR.

## Maps, Listings, and Reproducibility

Compiler output records the selected runtime and each binding decision. For
example:

```text
runtime: standalone
included: SArgs from <runtime:SYSLIB.ACT>
reason: call to BuildDisplayList has a four-byte argument frame
```

or:

```text
runtime: cart
SArgs -> $A0F5
SYS.PrintE -> $A46C
```

A local override records both the override location and the suppressed default
binding. Runtime source-map paths use the stable VFS names specified by the
loader note.

Runtime selection, dependency traversal, group ordering, layout, and map output
must be deterministic for identical inputs. The embedded VFS digest reported
by `actionc --version` makes the exact runtime source set observable.

## Implementation Slices

### Slice 1: Runtime configuration and unresolved helpers

- Add `Runtime::ActionCart` and `Runtime::Standalone` to the compiler request
  and CLI parsing, using `--runtime cart` and `--runtime standalone`.
- Keep the existing cart default during the transition.
- Carry logical helper requirements through MIR6502 instead of assigning cart
  addresses during materialization.
- Make classic runtime choice explicit rather than deriving it from segment
  storage.
- Report the runtime in maps and listings.

Suggested commit: `compiler: add explicit runtime selection`.

### Slice 2: Embedded standalone `SArgs`

- Load `SYSLIB.ACT` through the embedded runtime source provider.
- Resolve and relocate only `SArgs` for a large argument frame.
- Implement local symbolic `SET $4EE` precedence and suppress the default.
- Reject absolute external overrides under standalone.
- Run a copied-binary no-cartridge integration test.

Suggested commit: `runtime: selectively emit sargs`.

### Slice 3: Arithmetic helpers and dependency closure

- Add shifts, multiplication, division, and remainder.
- Derive routine calls and inseparable groups from resolved source.
- Preserve helper ABIs, effects, relocations, and zero-page requirements.
- Prove exactly-once inclusion and absence of unused routines.

Suggested commit: `runtime: link required arithmetic helpers`.

### Slice 4: Runtime-neutral `SYS`

- Finalize source syntax for external interfaces and runtime bindings.
- Add the one authoritative embedded `SYS` interface.
- Bind cart entry points without exposing their numbers as public constants.
- Bind standalone implementations from the split OSS runtime sources.
- Make qualified calls and compatibility-prelude aliases use the same stable
  symbols and ABI facts.

Suggested commit: `modules: bind std to selected runtime`.

### Slice 5: Runner and documentation

- Expose `actionc-run --runtime cart|standalone` as the canonical runtime
  selector.
- Keep `--no-cart` as the standalone convenience form and make `--cart PATH`
  imply the cart runtime.
- Document runtime license consequences and corresponding source.
- Add maps, listing explanations, and migration guidance for local helper
  definitions that are no longer necessary.

Suggested commit: `runner: align cartridge and runtime selection`.

## Validation Matrix

- MIR6502 plus cart and standalone;
- classic plus cart and standalone as support becomes available;
- identical `SYS` signatures and call semantics under both runtimes;
- cart helper calls retaining expected resident addresses;
- large call frames selecting exactly one embedded `SArgs`;
- small call frames containing no `SArgs` bytes;
- local symbolic `$04EE` override winning under both runtimes;
- standalone rejection of an absolute `$04EE` target;
- arithmetic helpers and transitive dependencies emitted exactly once;
- unused `SYS` and runtime routines absent;
- conservative address-taken and indirect-call retention;
- runtime call cycles emitted as deterministic groups;
- missing and ABI-incompatible bindings rejected before emission;
- standalone output running without the Action! cartridge;
- cart output retaining byte-compatible helper overrides;
- maps and listings reporting every runtime selection reason;
- stable GPL provenance and corresponding-source availability;
- identical inputs producing identical code, maps, and selection order.

## Deferred Runtime Features

- automatic or host-dependent runtime selection;
- hybrid cart/standalone policy;
- `--runtime none` as a strict no-support dependency check;
- user-supplied runtime packages;
- runtime version negotiation;
- dynamic loading or overlays;
- non-Atari standard-library providers;
- switching the default from cart to standalone before coverage is complete.

These can extend the provider model without changing `SYS` symbol identity or
allowing MIR6502 to recover bindings from source names.
