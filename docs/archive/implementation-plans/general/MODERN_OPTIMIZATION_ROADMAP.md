# Modern Optimization Roadmap

The north star for the modern profile is to move toward TAC and then SSA. Until
that pipeline exists, the direct code generator should still produce idiomatic
6502 code. Modern direct-codegen optimizations should therefore be local,
transparent, easy to disable, and shaped like backend rules that can survive an
IR refactor.

## Principles

- Keep modern-only rewrites behind `CodegenProfile::enables_modern_optimizations()`.
- Prefer slot, register, flag, and zero-page facts over fragile source-pattern
  hacks.
- Allow modern layout to diverge from original Action! implementation details
  when the observable language semantics stay the same. In particular, prefer
  per-routine centralized hidden storage over inline hidden bytes such as
  temporary loop caches and string literals in executable statement flow.
- Do not preserve original byte load/store order in modern profile merely for
  resemblance. Byte order, temporary placement, and register-transfer order are
  lowering choices. Modern may reorder them when Action! evaluation semantics
  and external ABI boundaries are preserved.
- Keep optimizations local or region-local. Avoid whole-program reasoning in the
  direct code generator.
- Log modern-only transformations so comparisons can classify expected wins.
- Preserve a clean path to TAC/SSA: direct-codegen rules should become future
  backend peepholes or lowering choices.

Avoid for now:

- inlining
- global common subexpression elimination
- whole-program optimization
- complex source-level rewrites
- large branch restructuring beyond safe short-range branch inversion
- optimizations that depend on incidental AST shapes unless they are stepping
  stones toward reusable lowering rules

## Phase 1: Return-Slot Lowering

Generalize the `Abs` insight into return-slot-first lowering rather than
recognizing names. Good first cases:

- `RETURN(param)` for single `BYTE`, `CARD`, and `INT` functions
- `RETURN(-param)` for `INT`
- simple conditional returns that can write directly to `$A0/$A1`
- parameter-local elision when ABI registers are enough and the parameter does
  not escape or get assigned

This should reduce local storage, copies, and unnecessary routine trampolines in
small helper functions.

The first conservative parameter-storage slice is implemented for the
modern/classic path. After ordinary lowering proves that a direct one- or
two-byte A/X frame has no body references, codegen removes both the parameter
cells and their entry capture stores. Address-taking, machine blocks, effect
annotations, current-location expressions, locals, hidden storage, and wider
SARGS frames keep the normal layout. Accepted and rejected decisions are
reported as `parameter-storage` proof attempts.

## Phase 2: Idiomatic Register Use

Expand the processor-state model cautiously:

- track A/X/Y byte provenance across straight-line code
- use `TAX`, `TXA`, `TAY`, and `TYA` when cheaper than a reload
- reuse known slot bytes when flags are irrelevant
- continue treating Action! zero-page ABI locations as pseudo-registers
- avoid reuse whenever compare/branch flags could be stale

These rules map naturally to future backend register allocation.

## Phase 3: Compare And Branch Cleanup

Build compare lowering as a small table of reusable shapes:

- byte equality
- word equality
- signed zero
- unsigned constant
- signed constant
- slot-vs-slot

Keep branch inversion short-range and safety-checked. Composite branch sequences
need explicit guards so labels are not accidentally retargeted.

## Phase 4: Array And Pointer Idioms

Use existing pointer/index tracking to reduce repeated setup:

- reuse prepared pointer addresses
- reuse X/Y indexes when still valid
- avoid repeated index scaling for the same element
- prefer stable `(zp),Y` forms where they are safe
- avoid redundant `LDY #0` / `LDY #1` when Y is known

The modern/classic two-byte element work is tracked in
`MODERN_CLASSIC_SCALED_CARD_ARRAY_INDIRECT_Y_IMPLEMENTATION_NOTE.md`.

## Phase 5: Observability

Modern optimization work should be explainable by the tooling:

- every modern-only transformation should have an optimization-log entry
- `action-compare` should classify expected modern wins separately
- focused torture tests should cover signed branches, return-slot lowering,
  pointer reuse, array index reuse, register transfer reuse, and flag-sensitive
  reloads

## Modern Routine Layout

Modern profile should not inherit every physical layout accident from the
single-pass cartridge compiler. Compatible profile keeps original placement for
ABI and probe comparison, but modern profile may choose a cleaner per-routine
layout.

Current policy:

- explicit locals remain in the routine storage block;
- explicit parameter cells remain by default, but a modern internal routine's
  direct one- or two-byte A/X frame may be removed when a parameter-storage
  proof finds no physical consumer or observable address;
- hidden routine-owned data is also emitted in that block, before the entry
  label/body;
- string literals used inside a routine are pooled in routine hidden storage
  instead of being emitted inline behind a local `JMP`;
- dynamic `FOR` end-bound caches are allocated in routine hidden storage instead
  of being embedded at the loop site;
- source/listing metadata should identify these bytes as modern hidden storage.

This is intentionally modest. It gives modern codegen a predictable storage
model without committing to whole-program storage coalescing yet. Later TAC/SSA
lowering can treat these hidden slots as routine-local temporaries and decide
whether to keep, reuse, stack-allocate, or eliminate them.

Routine entry is planned explicitly. Modern binds the entry label directly to
the executable prologue whenever the routine-boundary proof does not require a
patchable entry. Explicit parameters, locals, hidden storage, and local array
descriptors are emitted before that label and do not force a trampoline;
descriptor-backed array ranges remain registered with the program layout.
Public calls, `@routine`, machine-block routine addresses, and `RUNAD` all use
the same direct label, so address observability does not require an extra jump.
Compatible routine-name assignment still retains a writable trampoline operand.

On the modern/classic TN sample this relaxation removes 48 additional
fall-through entry jumps, or 144 raw bytes. The measured code segment shrinks
from 10,797 to 10,654 bytes (143 net): moving the routine/data addresses removes
one unrelated 1-byte register-reload optimization in `Format`, whose old
`$4848` address allowed `TXA` where the new `$47D0` address needs an immediate
load.

## Modern Internal ABI

Modern profile needs an internal ABI in addition to the public Action! ABI.
The public ABI remains the contract for externally visible calls: arguments are
packed from `$A0`, function results are available in `$A0/$A1`, and user code
or cartridge-compatible callers can rely on that behavior.

The internal ABI describes what the modern backend knows immediately before or
after a call/return. For example, a public byte result may still live in `$A0`,
but the internal ABI can also say that the same byte is currently in `A`. That
lets modern codegen store or forward the value directly instead of reloading it
from the public return slot. Longer term, inlined/internal-only calls can use
this model to skip public return-slot writes entirely when no public boundary is
crossed.

Current policy:

- compatible profile keeps the public ABI as the only required contract;
- modern profile may consume internal result locations such as `A` when facts or
  lowering prove them;
- modern/classic may consume direct incoming parameter bytes from `A`/`X` and
  omit their private cells when the `parameter-storage` proof succeeds;
- public return-slot materialization is still required at externally callable
  routine boundaries;
- inlining should lower into internal result locations first, then materialize
  the public ABI only if control reaches a public routine boundary.

## Virtual Temporaries

Modern profile should eventually allocate TAC/SSA values into a small hierarchy
of homes:

1. `A`, `X`, or `Y`;
2. Action!-owned zero-page scratch locations;
3. centralized routine-local storage spill slots.

The first foundation is the `VirtualTemp` model. It gives modern lowering a
nameable temporary with a width, purpose, and assigned home. The current
zero-page pool is deliberately conservative: byte temps may use `$AA`, and word
temps start at the known Action scratch pairs `$AC/$AD`, `$AE/$AF`, and
`$C0/$C1`. These homes are treated as volatile by default. A temp may survive a
call only when it is explicitly marked as preserved across a known call and the
callee effects do not write any byte of its slot.

This is not yet a local-variable allocator. The intended progression is:

- use virtual temps for compiler-generated expression temps first;
- add liveness and call-boundary checks;
- allow selected modern locals to live in zero page only when their live ranges
  and callee effects make that safe;
- spill everything else to centralized routine storage.

The zero-page temp pool is now configurable in the model. The default Action
modern pool intentionally preserves the original conservative candidate order:
byte temps can start at `$AA`, `$AC`, `$AE`, or `$C0`; word temps start at
`$AC`, `$AE`, or `$C0`. Larger modern pools should be expressed as sliding
ranges with explicit reserved ranges for OS, cartridge, runtime, and user-owned
zero-page symbols before the compiler starts placing locals there.

## Fact Finding

Modern direct codegen should ask for facts before it asks for rewrites. The
first fact layer is deliberately read-only and does not change emitted code.

Current facts:

- expression side effects distinguish read-only expressions from real routine
  calls, while treating Action array-call syntax as indexing rather than a call;
- value range facts identify exact constants and byte-width expressions;
- index addressing proofs classify byte-array/byte-index shapes as candidates
  for `absolute,Y` or `(zp),Y`, or mark wider element shapes as needing scaling;
- routine visibility facts identify routines that are retargetable through
  Action routine assignment and therefore should not be treated as ordinary
  internal-only routines;
- call-boundary proofs wrap the zero-page temp survival check against known
  callee effects.

These facts are intentionally conservative. They are meant to become inputs to
future TAC/SSA lowering and modern backend decisions, not a new pile of
source-pattern shortcuts.

### Proof-Guided Lowering Status

Update: 2026-07-18.

The first narrow proof consumers are wired into direct codegen:

- value-availability proofs feed scalar call-result byte loads and the
  assignment fallback;
- index-address proofs feed inline byte-array scalar-index loads, including
  call-argument loading before the generic lvalue fallback;
- parameter-storage proofs remove private one- or two-byte parameter cells and
  their capture stores only after lowering leaves no physical storage consumer.

`actionc-emit --emit-proofs --profile modern <file.act>` prints accepted proof-guided
lowering events with the source location, output address, routine, proof kind,
and summary. This is intentionally an accepted-proof event stream rather than a
complete solver trace.

Use `actionc-emit --emit-proof-attempts --profile modern <file.act>` when investigating
why a proof-guided lowering did not fire. It prints the same location context
with an `ok` or `reject` status and a short rejection reason. This remains a
debugging view: normal proof output should stay focused on accepted lowering
events.

Focused sweep after these consumers:

- `fixtures/stress/advanced_pointers.act`: compiles; no accepted proof events
  in the current narrow paths.
- `fixtures/stress/arrays.act`: compiles; reports an `index-address` event in
  `LocalWork`.
- `samples/tn/modern/TN.ACT`: compiles; reports `index-address` events in
  `DrawWinFrame` and `Copy`, plus `value-availability` events for call-result
  byte loads in several routines.
- `samples/toolkit/original/extracted/ALLOCATE.ACT`: still hits an existing
  unsupported-expression codegen limitation at `2426..2435`; this is not a new
  proof-regression signal.

## ABI Argument Materialization Audit

Audit date: 2026-05-20.

Modern source listings were generated for TN, the Action Toolkit sources that
currently compile in modern profile, and the pointer/string/record/array stress
tests. The scan looked for call-argument staging patterns where `$A0`, `$A1`, or
`$A2` is written shortly before being loaded into the ABI register:

- `$A0` -> `A`
- `$A1` -> `X`
- `$A2` -> `Y`

Remaining candidates by listing:

- `TN`: `$A0=20`, `$A1=24`, `$A2=1`
- `CIRCLE`: `$A0=0`, `$A1=8`, `$A2=8`
- `SORT`: `$A0=4`, `$A1=4`, `$A2=3`
- `advanced_pointers`: `$A0=3`, `$A1=0`, `$A2=0`
- `PRINTF`: `$A0=0`, `$A1=2`, `$A2=0`
- `TURTLE`: `$A0=1`, `$A1=1`, `$A2=1`
- `PMG`: `$A0=0`, `$A1=0`, `$A2=1`
- `IO`: `$A0=0`, `$A1=1`, `$A2=0`

Immediate adjacent `STA`/reload pairs are mostly `$A1 -> X` and `$A2 -> Y`.
Examples include:

- `STA $A1 ; LDX $A1` for second-byte call arguments, often when the value was
  just computed in `A`.
- `STA $A2 ; LDY $A2` for third-byte call arguments after indexed byte loads.

The next safe optimization should therefore be register-targeted argument
forwarding for later ABI bytes:

- when loading call argument byte 1 into `X`, emit `TAX` instead of
  `STA $A1 ; LDX $A1` if the computed byte is still in `A`;
- when loading call argument byte 2 into `Y`, emit `TAY` instead of
  `STA $A2 ; LDY $A2` if the computed byte is still in `A`;
- keep the existing store when `$A1/$A2` must survive for a later expression,
  return capture, or post-call use.

This is a better next target than more first-byte `$A0` work. Most `$A0`
materialization candidates in TN are part of wider staged call/return sequences
where the value is deliberately preserved across loading other argument bytes.

### TN `$A1 -> X` Classification

Follow-up audit after final-argument forwarding: TN still has 22 `$A1` stores
that are reloaded into `X` within the same call setup window.

No sampled context needed `$A1` itself after argument setup. The remaining cases
fall into three implementation-shaped buckets:

- 17 cases are word argument high-byte staging. The code loads the high byte of
  a pointer/CARD expression into `A`, stores it to `$A1`, loads/stores the low
  byte through `$A0`, then reloads `$A1` into `X` before the call. These should
  become `TAX` immediately after the high-byte load, followed by the existing
  low-byte load into `A`.
- 2 cases are word-expression high-byte staging after carry propagation, for
  example `Value(v(i)+1)`. These are the same core opportunity as the previous
  group, except the high byte comes from `ADC #$00` instead of `(ptr),Y`.
- 2 cases are scalar second-argument expressions where `A` already holds the
  final second byte, then the compiler loads later `Y`/`A` arguments before the
  call. These need a small non-final register-preload guard: `TAX` is safe when
  later argument setup does not clobber `X`.
- 1 case is zero-extension after a helper return. It stores `0` to `$A1`, then
  reloads `X` for a word argument. This should be handled by direct high-byte
  register loading, e.g. `LDX #0`, not by memory staging.

The best next implementation target is therefore not a broad liveness system
yet. Start with **word argument high-byte forwarding** in staged call setup:

- when a two-byte argument contributes register bytes `A/X`, load or compute
  the high byte first and `TAX`;
- then load or compute the low byte into `A`;
- do this only when the high byte is not needed as a memory value and later
  setup cannot clobber `X`.

This should cover most TN cases and also maps cleanly to future backend
lowering: it is just direct register construction for a word ABI argument.

## Recommended Next Step

Start with return-slot-first lowering as reusable direct-codegen rules:

1. `RETURN(param)` for single scalar functions.
2. `RETURN(-param)` for signed `INT`.
3. Conditional return shapes only when they fall out naturally from those lowerings.

This improves modern output now without building machinery that fights the
future TAC/SSA design.
