# MIR6502 unexposed source-object storage qualification

2026-11-15. Implements the
[design](../../MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md) and
[plan](../../MIR6502_UNEXPOSED_OBJECT_STORAGE_IMPLEMENTATION_PLAN.md).

## Implementation

- `85ea1332`: share routine-qualified address-control facts independently of
  inline/ZP placement, retaining source roles and historical exposure.
- `64c4b8d5`: separate ordinary native pointee writes from physical home/pair
  clobbers; retain final physical inventories and check both placements.
- `15da9bbd`: admit native byte/word indexing, scalar array-parameter pointer
  cells and constant byte shifts 0–8. Keep unsupported/helper/unknown-call gates.

Address control does not prove initialization or physical safety. Public write
summaries for pointer writers remain Unknown. The existing combined proposal,
strict whole-image acceptance, preservation replay and exact rollback remain.
An unresolved explicit-observation inventory rejects a new pointer writer during
discovery, before adding it to the combined trial. Concrete overlap checks use
both final layouts and retained witnesses. No retry/subset search was added.

## Focused validation

Slice 1 ran the address-control group, five local controls, fifteen
capture/handoff controls and five scoped-observation controls. Slice 2 ran both
then-current object-storage groups, the five existing local controls and five
scoped-observation controls. The final implementation passed:

```text
cargo test --lib object_storage_                                      4 tests
cargo test --test mir6502_cross_call_homes object_storage_            2 tests
cargo test --manifest-path tools/vm-runtime-tests/Cargo.toml \
  --test cross_call_homes object_storage_                             1 group
```

The pipeline tests establish actual ZP selection, source-ZP avoidance, a bounded
trial, LF/CRLF image identity and retention of an independent caller when explicit
assembly observations are undecodable. The VM group covers placement off/on in
ActionCart and Standalone, byte/word swaps across an ordinary leaf call, a word
crossing a page, repeated invocations, argument/result values, array/source-ZP
sentinels and stack balance. Zero/eight-count shifts have independent value
oracles. Unit controls cover logical exposure, stale facts, initialization,
word-lane clobbers, old/new physical ranges and unsupported shifts/helpers.

Also passed on the final implementation: the existing unsupported-call,
exposed-object and exhausted-ZP fallback integration test; exact selected-input,
executable and physical-map rollback; all four multiple-call unit controls,
including later-site argument and high-lane write rejection. Prebuilt test
binaries were used for these existing controls. No full suite was run.

The shared target used opt-level 1, no debug information/incremental compilation
and two build jobs. No frontend/NIR contract or emission recipe changed.

## TN result

Optimized MIR6502 TN, ActionCart at `$3000`, is **9,838 bytes**, down **8 bytes**
from the recorded `7ea9e7ab` baseline of 9,846. The TN/LIB source hashes and
configuration match that baseline. One completed traced generation measured
19.994 seconds and sampled 545,152 KiB resident memory. This excludes Rust
compilation and the diagnostic emission replay; it is not a peak-RSS measurement
or a performance benchmark series. An initial collector attempt was interrupted
while extending its retained-MIR output and produced no qualification artifacts.

| Routine | Code bytes before → after | Mapped ordinary bytes before → after | Mapped ZP bytes before → after |
| --- | ---: | ---: | ---: |
| Instr | 60 → 55 | 3 → 0 | 0 → 3 |
| Sort | 250 → 250 | 3 → 3 | 0 → 0 |
| Fnamecmp | 105 → 105 | 0 → 0 | 4 → 4 |
| Ord | 51 → 51 | 0 → 0 | 5 → 5 |
| Items | 37 → 37 | 0 → 0 | 3 → 3 |
| FindItem | 53 → 53 | 0 → 0 | 3 → 3 |
| Next | 30 → 30 | 0 → 0 | 0 → 0 |

Instr is the only routine whose code/home counts change among all 101 routines.
Its array-reference parameter's pointer cell moves to `$A0/$A1`, and its byte
parameter to `$A2`. Three shorter captures, one shorter load and one shorter
comparison save five code bytes; removing the inline parameter storage saves
three data bytes. See [before/after assembly](final/instr-before-after.asm).
Other code/home counts remain unchanged, although layout addresses relocate.
Mapped home counts include parameter/local map entries, including mapped spills;
they do not count all working storage or source globals.

Sort passes the new body gate but is rejected **before proposal** with
`unresolved explicit observation`. Its Fnamecmp call independently passes the
entry/write contract. TN contains six machine-coded menu-data bodies (`drives`,
`delcancel`, `density`, `yesabort`, `lockunlock`, `yescancel`) with bytes the
instruction scanner cannot decode. The conservative explicit-observation
inventory therefore remains incomplete. Distinguishing data from executable
machine bodies is outside this slice; no exception was added to claim a saving.
Sort's `h` and mapped gap spill remain inline. Its source globals `i/j` remain
at `$E0/$E1`.

Ord, Items and FindItem still enter the one combined ordinary-call proposal and
retain their previous savings. The proposal goes from 9,918 to 9,838 bytes and
passes normal final verification and preservation replay. Sort's missing proof
does not discard these independent candidates.

All 40 leaf-candidate entry facts remain unchanged. Instr's Exact write footprint
now names its selected fixed-ZP parameter bytes; it still includes global `i`
and scratch `$AC/$AD`. Its reads remain Unknown, X preservation remains valid,
and both Handle call checks pass. All other logical entry/exit summaries are
unchanged when relocated numeric addresses are excluded from the comparison.
Fnamecmp and Next retain their contracts. The static instruction audit finds
no uncovered direct accesses, unresolved Exact identities or contradicted X/Y
preservation claims. This scan is an audit aid, not a semantic execution proof.

## Work and artifacts

Materializations remain **6**, emission invocations **145**, internal emission
probes **870**, and preservation replays **17**. There is still **one** ordinary-call
home trial; the leaf and handoff trials are also unchanged. Source-storage
inventory builds increase from 56 to 60 because the four already-required final
emissions now retain their physical maps. No additional materialization or
emission was introduced. See every counter and all decisions in the
[comparison](final/comparison.json).

The final XEX SHA-256 is
`9a94d4aa9c28201a6394cccb7dec7e2777165538bc96dd71ecaad6658ce3b9a3`.
The [receipt](final/receipt.json) records compiler/source/config evidence and
hashes for the full selected MIR, image, map, relocations and assembly. The hook
was removed, and diagnostic emission replay reproduced the final image and
relocations exactly. Raw artifacts are retained under
`build/unexposed-object-storage/final/`; compact versioned evidence is:

- [Facts](final/facts.compact.json) and [instruction audit](final/audit.json).
- [Control MIR](final/controls.mir) and [control assembly](final/controls.asm),
  including Sort, Fnamecmp, Ord, Items, FindItem and Next.
- [All routine footprint changes and work counts](final/comparison.json).

TN qualification covers generated code and compiler facts. No TN UI execution
or full repository test suite is claimed.
