# AGENTS.md

## Project context

This repository implements `actionc`, a modern compiler for the Action! language.
The `nir` module contains the Normalized Intermediate Representation. Treat
`nir` as the current code home for NIR hardening.

The long-term pipeline is:

```text
Action source -> AST -> semantic model -> SemIR -> NIR -> MIR6502 -> emission
```

## Architecture rules

- SemIR owns Action! meaning: name resolution, typing, lvalue legality, callable
  facts, array/routine disambiguation, record facts, pointer facts, and source
  control-flow meaning.
- NIR owns normalized typed computation: explicit blocks, temps, loads, stores,
  casts, compares, branches, calls, static data references, storage facts, and
  conservative effects.
- MIR6502 owns target strategy: byte/word expansion, 6502 addressing choices,
  zero-page decisions, A/X/Y/flags use, ABI placement, helper selection, and
  target-specific peepholes.
- Emission owns final instruction/data writing, labels, patching, maps, listings,
  and proof hooks.
- Do not make MIR6502 look back into SemIR to recover facts that should be in
  NIR.
- Do not put source-language semantic decisions into MIR6502 or emission.
- Do not add optimizer passes over legacy/stringly NIR forms.
- When removing or replacing a legacy NIR shape, tighten the verifier so the old
  shape cannot silently reappear.

## NIR Migration Rules

Verifier-clean NIR must eventually contain no executable:

- `NirOperand`;
- expression-summary strings;
- raw source strings;
- unresolved names;
- `Symbol(String)` storage identity;
- field names instead of byte offsets;
- index syntax strings;
- indirect callee target strings;
- metadata operations inside executable blocks;
- unknown/open control-flow boundaries;
- string labels as the long-term CFG identity.

Use stable IDs and fact tables for MIR-relevant entities whenever practical:

- routines;
- blocks;
- temps;
- params;
- locals;
- globals;
- statics;
- machine blocks;
- signatures.

Keep display names and source syntax only as debug/printing/source metadata, not
as executable semantics.

## Optimization rules

- Optimizer passes may run only after NIR verification.
- Every optimization pass must preserve verifier-clean NIR.
- Prefer small, obviously safe passes first: CFG cleanup, constant folding, copy
  propagation, branch simplification, unreachable block removal, and dead temp
  elimination.
- Be conservative around calls, absolute memory, OS/runtime calls, hardware
  registers, pointer dereferences, and machine blocks.
- Treat calls and machine blocks as ordering barriers unless structured effects
  prove a narrower behavior.
- Do not perform aggressive alias-sensitive optimization until storage identity,
  memory effects, and dominance/use-def validation are strong enough.

## Implementation discipline

- Make migrations as small vertical slices.
- Keep existing fixtures green unless the task explicitly updates the expected
  output.
- Preserve readable printed IR output when replacing internal string identities
  with stable IDs.
- Do not perform broad module renames unless explicitly requested.
- Do not mix unrelated cleanup, optimization, and emission changes in one patch.
- If a semantic behavior is unclear, document the uncertainty and prefer an
  explicit unsupported diagnostic over guessing.
- Do not add special cases for individual sample programs; add general compiler
  behavior with focused regression coverage.

## Cross-platform test fixtures

- Treat checked-out text from `include_str!` and `read_to_string` as potentially
  CRLF, even when local files use LF. Windows CI has repeatedly exposed this in
  README output extraction and multiline Action! source instrumentation.
- Normalize text with `.replace("\r\n", "\n")` before newline-sensitive
  `split_once`, `matches`, `replace`, or expected-output conversion to ATASCII.
  Prefer `.lines()` when only individual lines matter. Reuse existing snapshot
  normalization helpers where applicable.
- Normalize only host text whose newline convention is irrelevant to the test.
  Preserve exact bytes for binaries, ATASCII fixtures, and tests specifically
  checking line-ending behavior; do not use broad whitespace trimming to hide
  output differences.
- When adding or changing newline-sensitive fixture handling, verify both LF
  and CRLF inputs through the actual parsing or instrumentation path. For
  embedded fixtures, use an isolated CRLF checkout and rebuild the affected
  tests if needed. A passing run with local LF files alone is insufficient.

## Required checks

After changing NIR, semantic lowering, verifier, printer, or related code,
run the relevant checks before submitting the change:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
cargo test
```

When a task introduces NIR-specific fixtures or commands, also run their matching
checks, for example:

```sh
cargo test nir_fixtures_match_snapshots
cargo run --bin actionc-nir-sweep -- fixtures/nir
```

If a fixture changes, explain whether the change is an intentional IR contract
change, a printer-only change, or a bug fix.

## Documentation expectations

- Update the relevant design document when changing an IR boundary.
- Keep boundary documents focused on contracts and invariants, not implementation
  diary entries.
- Prefer documenting rejected legacy forms and verifier guarantees explicitly.
- When adding a new NIR form, document who owns it: SemIR, NIR, MIR6502, or
  emission.

## Review Checklist

Before considering a NIR migration patch complete, check:

- Does verifier-clean IR become stricter or at least no weaker?
- Did any executable path gain a new stringly or summary-based dependency?
- Can MIR6502 consume the new shape without consulting SemIR?
- Are calls, machine blocks, absolute memory, and pointer writes still handled
  conservatively?
- Are printed names still readable even if internal IDs changed?
- Are tests or fixtures updated for the specific behavior changed?
