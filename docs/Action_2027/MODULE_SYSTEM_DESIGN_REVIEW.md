# Review: Action 2027 Module System Design

Review of [`MODULE_SYSTEM_DESIGN.md`](./MODULE_SYSTEM_DESIGN.md) on the
`Action-2027` branch. Reviewer: @copilot (via @mkur), 2026-08-18.

## Disposition

The review has been resolved with the accepted decisions recorded in three
focused contracts:

- [`MODULE_SYSTEM_DESIGN.md`](./MODULE_SYSTEM_DESIGN.md) — language syntax,
  visibility, qualification, and compiler IR boundaries;
- [`MODULE_LOADER_AND_VFS.md`](./MODULE_LOADER_AND_VFS.md) — file resolution,
  embedded sources, and the single-binary invariant;
- [`RUNTIME_INTERFACE_AND_STANDALONE.md`](./RUNTIME_INTERFACE_AND_STANDALONE.md)
  — cart/standalone bindings and selective runtime inclusion.

The review text below remains unchanged as the rationale and decision record.

## Overall assessment

The design is unusually well thought through: it clearly separates SemIR
ownership of module identity from NIR/MIR6502, treats `SYS`/`ATARI.*` as
embedded Action source rather than a parallel Rust table, and makes the
single-binary invariant testable. The main risks are **under-specified
corners** (parsing ambiguities, error recovery, tooling surfaces) and a few
**semantic decisions worth revisiting** before Slice 2 locks them in.

---

## High-impact issues

### 1. `MODULE` header ambiguity vs. legacy bare `MODULE`

The rule "a named module has a name and `ENDMODULE`; a bare module has
neither" is only reliable if the parser can decide *before* it consumes
declarations. Two concrete problems:

- `MODULE DEMO` followed by (accidentally) no `ENDMODULE` at EOF — is that
  a "malformed named module" or "bare `MODULE` followed by an identifier
  used as a type name in a later declaration"? The current text does not
  say.
- A file that begins `MODULE DEMO.PLAYER` but never uses `PUBLIC` and never
  has `ENDMODULE` will parse very differently in the two interpretations,
  silently changing visibility.

**Proposal:** state explicitly that (a) a `MODULE` header followed by an
identifier commits the file/region to *named-module* parsing; missing
`ENDMODULE` is a hard error with a fix-it, never a fallback to legacy;
(b) legacy bare `MODULE` is recognized only when the next non-trivia token
is a *declaration keyword* (`BYTE`, `CARD`, `PROC`, `FUNC`, `TYPE`,
`DEFINE`, `MODULE`, EOF). Add these to the Slice 1 diagnostics list.

### 2. Multiple named modules per file — the "root source" carve-out is dangerous

The note says "a root source may contain several modules for tests and
small programs" but also that automatic path lookup is one-module-per-file.
This creates a two-tier language where the same construct means different
things at the root vs. anywhere else, and it interacts badly with the
case-insensitive path→file mapping.

**Proposal:** either

- **restrict to one named module per file always** (recommended; simplest
  invariant, matches the file-path mapping), and use `INCLUDE` or a test
  harness for multi-module tests; or
- **allow multiple named modules per file everywhere** but require that
  only one may match the file's derived path when the file is loaded via
  `USE`; extra modules are only visible within the same file.

Pick one and document it. The current middle position will produce
inconsistent tooling.

### 3. Grouped `PUBLIC` is a footgun

Section "Public declarations" says grouped declarations share visibility
and recommends splitting. In Action! the grouped form is idiomatic
(`BYTE a, b, c`), and the resident tables you plan to migrate use it
heavily. A single accidental `PUBLIC` on a group can leak private state.

**Proposal:** allow per-name visibility inside a group, e.g.
`BYTE PUBLIC a, b, PUBLIC c`, *or* require groups to be all-public /
all-private and produce a warning (not just a stylistic note) whenever a
mixed-intent group is used by another module. Also state the
interaction with `VOLATILE` groups (does
`PUBLIC VOLATILE BYTE A=$D000, B=$D001` propagate both qualifiers to every
name? — probably yes, say so).

### 4. `PUBLIC` on `PROC name=$ADDR(...)` is under-specified

The `SYS` sketch uses `PUBLIC PROC PrintE=$A46C(...)`. Two things are
unclear:

- Is `$A46C` part of the *public interface* (visible to using modules, usable
  in `SET $04EE=SYS.PrintE`)?
- Under `--runtime standalone`, what happens to that address? Silently
  ignored? Diagnostic if referenced numerically?

**Proposal:** define the fixed-address form as a *cartridge binding*, not
part of the interface. Add a source-level "binding" concept — e.g.
`PUBLIC PROC PrintE(CHAR ARRAY text) BINDING cart=$A46C, standalone=Std_PrintE` —
and defer the exact spelling to the runtime-interface design that the note
already promises. Say what happens if a binding is missing for the
selected `--runtime`.

### 5. `USE ALL FROM` + compatibility-prelude coalescing needs a formal rule

"Both bind the same `SymbolId`, so no duplicate diagnostic" is right for
`SYS`, but silently generalizes to a rule: *equal `SymbolId` from two
`USE` paths coalesces; equal names with different IDs are an error.*
This should be stated as the general rule, not just applied to `SYS`.

**Proposal:** add a subsection "Alias identity vs. name identity: two
bindings for the same `SymbolId` coalesce silently; two bindings of the
same name for different `SymbolId`s are always an error." That also gives
a clean answer to future re-export.

### 6. Dependency cycles are banned but hardware modules use one another

`ATARI.VIDEO` uses `ATARI.ANTIC`, `ATARI.GTIA`, `ATARI.OS` — fine as a
DAG. But `SYS` is likely to grow internal cross-references, and the note
explicitly says cycles are diagnosed with the "complete module chain"
without an escape hatch. Real embedded runtimes (`SYSIO` ↔ `SYSMISC` etc.)
may need cycles.

**Proposal:** commit to *declarations-first, bodies-later* module loading
(mentioned once, but not as a plan): after Slice 4, every module's public
interface is fully computed before any body is resolved. That eliminates
most benign cycles and leaves a real error only for *interface* cycles.
Add this to the validation matrix.

### 7. `SET $04EE=r_Par` compatibility model is contradictory

The note says user `SET $04EE=r_Par` is a "compatibility escape hatch" but
also that MIR6502 owns helper selection and automatically resolves
`SArgs`. If both mechanisms are active in one program, which wins? What
about a program that does `SET $04EE=r_Par` under `--runtime standalone`?

**Proposal:** state the precedence explicitly:

- Under `--runtime cart`: user `SET` wins (bytewise compatibility with
  1984 sources).
- Under `--runtime standalone`: user `SET` to a fixed slot is a warning,
  and the compiler still emits an embedded `SArgs` unless the user also
  declares their `PROC r_Par` as `PUBLIC` and uses it; specify what
  "declaring a helper" looks like or drop the escape hatch for standalone
  entirely.

### 8. Missing: what does `--module-path` actually accept?

Slice 4 lists it but says nothing about semantics: multiple paths?
later-wins or earlier-wins? does it shadow reserved roots (the doc says
no — repeat that here)? relative to CWD or the root source? env-var
equivalent?

**Proposal:** one paragraph specifying: repeatable, earliest-match wins,
resolved relative to the invoking CWD, cannot introduce paths under
reserved roots (`SYS`, `ATARI`), and *not* consulted from
`ACTIONC_MODULE_PATH` or any env var (keeps the single-binary invariant
meaningful).

---

## Medium-impact gaps

### 9. Diagnostic taxonomy is uneven

Some sections list specific diagnostics ("duplicate alias, malformed
path…"), others just say "an error." Please give every rule a stable
diagnostic name/code so tests can pin them:

- `E-MODULE-NAME-MISMATCH` (declared vs. requested identity)
- `E-MODULE-CYCLE`
- `E-MODULE-PRIVATE-MEMBER` vs. `E-MODULE-UNKNOWN-MEMBER` (already
  distinguished — good)
- `E-OPEN-USE-COLLISION`
- `E-RESERVED-ROOT-SHADOW`
- `E-BINDING-MISSING-FOR-RUNTIME`

### 10. Case-insensitive identities on case-sensitive filesystems

"Compares case-insensitively" but "maps deterministically to a lowercase
source path" leaves a hole: what if the user's file is `Player.act`? On
Linux, `game/entities/player.act` won't be found. Say explicitly that the
on-disk filename must be lowercase for automatic lookup, and produce
`E-MODULE-FILE-CASE-MISMATCH` otherwise (don't silently accept whatever
the FS returns — that's a Windows/mac vs. Linux portability trap).

### 11. `INCLUDE` interaction with named modules is fuzzy

Can a `MODULE FOO … INCLUDE "helpers.act" … ENDMODULE` add declarations to
`FOO`? If yes (implied but not stated), what's the visibility default of
textually included declarations — private-by-default (module rule) or
legacy (global-by-default)? What about `PUBLIC` inside the include?

**Proposal:** state that inside a named module, `INCLUDE` behaves as a
textual insertion *of module-scoped declarations*: private-by-default,
`PUBLIC` allowed. Outside any named module, `INCLUDE` retains full legacy
behavior. Add a test case.

### 12. Circular `PUBLIC` types across modules

Nothing forbids `RECORD`s in module A referring to types in module B and
vice versa. With interface-first loading (see #6) this is fine; without
it, layout computation can loop.

**Proposal:** commit to two-phase name resolution (identity + signature
first; body/expression resolution second) and add a validation-matrix
entry for mutually-recursive `TYPE`/`RECORD` across modules.

### 13. Debugger / listings symbol scheme is inconsistent

Listings show `ATARI.ANTIC.WSYNC`, MADS labels use `atari_antic_wsync`,
and the note mentions "stable suffixes only when sanitization would
collide." Two collisions worth calling out:

- `Foo.Bar_Baz` and `Foo_Bar.Baz` both sanitize to `foo_bar_baz`.
- User project modules chosen to shadow a MADS label
  (`atari_antic_wsync` as a user identifier).

**Proposal:** reserve `atari_` and `std_` MADS-label prefixes, and specify
the collision suffix as a deterministic short hash of the `SymbolId`
(never a serial counter, which changes across builds).

### 14. Nothing about incremental compilation or interface stability

This is fine for now, but the note takes several decisions (whole-program,
no separate objects, cycle-diagnosed loader) that will bite later. One
paragraph explicitly saying "incremental compilation, separate `.aci`
interface files, and a linker are non-goals for Action 2027; a future
evolution should preserve `SymbolId` stability across compilations" would
set expectations.

### 15. Missing: what modules can *do* to the compilation model

No mention of:

- Whether `DEFINE` in a used module is macro-substituted in the
  using module's source (probably no — say so) or resolved as a semantic
  constant (probably yes).
- Whether inline `[ … ]` assembler in module A can reference
  `B.SomeLabel`. The note *mentions* "inline-assembler symbols" in the
  SemIR resolution list but doesn't say if qualified names work inside
  `[ … ]`.
- Whether `SET addr=X` can use a qualified `X` (implied yes by validation
  matrix; state it in the syntax section).

### 16. Validation matrix items to add

Beyond what is already listed:

- `RECORD` field access on a value of a public type from a used module (must not
  be confused with `module.member`).
- `USE` inside a `MODULE` block vs. at file top level (allowed only
  inside, or both?).
- A negative test: user file declaring `MODULE SYS` must fail with
  `E-RESERVED-ROOT-SHADOW`, not a duplicate-module error.
- Determinism: identical inputs → byte-identical embedded-VFS image and
  byte-identical MADS labels.

---

## Smaller edits

- **§USE clauses:** "Without `AS`, the last component is the alias" — say what
  happens for `USE SYS` (last component == whole name), and forbid
  `USE SYS AS SYS` as redundant (warning, not error).
- **§Hardware Modules:** `DLIST=$D402` is exposed as `CARD`, but
  `DLISTL/DLISTH` are actually at `$D402/$D403`; note the endianness
  constraint you're relying on for 6502 and say `PUBLIC VOLATILE CARD`
  implies little-endian *low-byte-first* pair — some ANTIC pairs are not
  that.
- **§Compatibility prelude:** name the flag/toggle (e.g.
  `--legacy-prelude=on|off`) even if the "off" path is deferred; naming it
  now prevents flag churn.
- **§CLI:** you already write `--runtime standalone` / `cart`. Please also
  spell out that `--runtime standalone` implies `--backend mir6502` (or
  errors if a different backend is chosen), because the whole "standalone"
  story is MIR6502-specific.
- **§Embedded VFS:** the content digest is a great idea; also emit it in
  the build banner (`actionc --version` shows VFS digest) so bug reports
  can pin the exact embedded corpus.
- **§Legacy `MODULE`:** worth an example of a *mixed* file: legacy
  declarations, then a named module — is that allowed? (Recommended:
  **no**; require a file to be either entirely legacy or entirely
  named-module.)

---

## Suggested reorganization

The document mixes design intent, syntax, semantics, architecture, and
implementation plan. Consider splitting into three sibling notes to keep
review tractable:

1. `MODULE_SYSTEM_DESIGN.md` — motivation, principles, syntax, semantics,
   validation matrix. (The language contract.)
2. `MODULE_LOADER_AND_VFS.md` — resolver, embedded VFS, `--module-path`,
   single-binary invariant, source-provider interface. (The
   compiler-internal contract.)
3. `RUNTIME_INTERFACE_AND_STANDALONE.md` — `cart` vs. `standalone`, `SYS`
   bindings, helper selection, GPL provenance, fragment format. (The
   design already says this belongs in a dedicated runtime-interface
   note — start it now, since Slices 6–7 depend on it.)

That split also matches the natural review audiences: language users,
compiler contributors, and runtime maintainers.

---

## Suggested near-term actions

- Open focused issues for items **1, 2, 3, 4, 7, 11** (the ones that will
  change source code or produce different diagnostics).
- Draft a PR against `Action-2027` that (a) adds the diagnostic-name table
  (#9), (b) tightens the `MODULE` header rule (#1), (c) commits to
  interface-first loading (#6/#12), and (d) splits the runtime-interface
  section into its own note (reorg).
