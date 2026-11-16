# Direct ZP homes and live-call exclusions: qualification

2026-11-16. Implements the invocation-placement part of the
[lifetime design](../../MIR6502_UNEXPOSED_OBJECT_STORAGE_DESIGN.md#zp-home-lifetimes-and-persistent-storage)
and its [four-slice plan](../../MIR6502_ZP_HOME_LIFETIMES_IMPLEMENTATION_PLAN.md).
Dedicated persistent reservations and working copies remain follow-on work.

Optimized MIR6502 TN, ActionCart at `$3000`, shrinks from **9,815 to 9,811 bytes**.
The gate-off image exactly matches the retained `56526393` baseline. Both
measurements use identical sources and compiler code, with only
`enable_zp_home_lifetimes` changed. The optimized default now enables it.
The conservative default remains disabled.

Only `PrintNonzeroDigit` changes size:

| Component | Before | After |
| --- | ---: | ---: |
| Instructions | 47 bytes | 44 bytes |
| Inline homes | 2 bytes | 1 byte |
| `b` | `$33FA` | `$33FA` |
| `divisor` | `$33FB` | `$A2` |

`divisor` dies before Putchar; `b` must survive it. Three accesses gain ZP
encoding and one inline byte disappears. See the actual disassembly
[before](print-nonzero-digit-before.asm) and
[after](print-nonzero-digit-after.asm), and the
[measurement receipt](qualification.json) for source/configuration hashes.

The independent live-call control shrinks from **248 to 225 bytes**. A home
defined after an unqualified call and a parameter dead at that call can move;
a sibling live across the same call stays inline. Separate caller subtrees
reuse the same pair. An exposed parameter and an entry-dependent local no
longer prevent eligible siblings from receiving homes.

## Bounds and unchanged controls

Per-routine allocated ZP peaks at seven bytes in Sort, unchanged. The union of
mapped homes and virtual-slot allocations stays at eleven distinct bytes:
`$A0–$A3`, `$E2–$E5`, `$EA–$EC`. These are static allocation counts, excluding
source-owned globals and transient ABI/helper traffic, not a runtime high-water
measurement. The new home reuses `$A2`.

| Routine | Instructions, before = after | Inline homes, before = after |
| --- | ---: | ---: |
| Ord | 51 | 0 |
| Items | 37 | 0 |
| FindItem | 53 | 0 |
| Strcpy | 20 | 0 |
| Fnamecmp | 105 | 0 |
| Sort | 238 | 0 |
| Next | 30 | 0 |
| PopUp | 261 | 11 |
| Copy | 673 | 58 |

Sort retains `h` at `$E2/$E3` and the loop-carried spill `gap` at `$EC`.
Discovery excludes known numeric ZP observations for native pointee writers.
Otherwise Sort could select `$A2/$A3`, conflict with MovePage's explicit pointer
fetch, and invalidate the whole combined trial. The prefilter uses the existing
observation inventory; final layout checks, preservation replay and exact
rollback remain authoritative. There are no subset retries.

PopUp's first remaining admission blocker is call index 5, Range's five-byte
SArgs capture. This slice validates ordinary native entry lanes; it does not
add SArgs or observable assembly entry contracts. All its homes therefore stay
inline, including short-lived `item`. Independently, `menu` reads its previous
invocation's value in `Push(menu)`, and `c` is exposed through `@c`. `mp`, `spc`,
`ch`, `d` and `limit` also retain the selected inline bindings; removing Range's
blocker alone would not certify their other live-call obligations.

## Work and validation

Both TN measurements use exactly one combined cross-call trial, six
materializations, 146 emission invocations, 876 inner emission probes and 17
late-cleanup preservation replays. All recorded work counters match. Diagnostic
emission replays are byte-identical to their selected artifacts.
The normal CLI also emits the qualified 9,811-byte image; MADS reconstructs it
byte-for-byte from both LF and CRLF versions of the emitted assembly listing.

All 56 placement unit tests, 46 selected integration tests and 22 VM tests
passed; target counts are recorded in the receipt. Two optional MADS matrix
tests remained ignored; the focused TN listing check above passed. Coverage
includes mixed
exposure/persistence, word lanes, setup clobbers, dead and live boundaries,
repeated and recursive calls, alternating subtrees, numeric observations,
parent gates, whole-image acceptance and exact rollback. The new source-file
controls run with LF and CRLF through the actual loader, with placement off/on
in ActionCart and Standalone.

The TN execution control invokes the actual compiled PrintNonzeroDigit in both
runtimes. It captures Putchar's argument and deliberately overwrites the dead
divisor home, checking the returned remainder and stack balance. Screen output
is stubbed; this is a focused routine oracle, not an interactive TN session.
Standalone retains its 10,025-byte selected image and inline divisor: final
observation replay rejects the combined trial because generated
`ACTION.RUNTIME.SYSLIB::Error` has an unresolved machine-block operand inventory.
The runtime scanner is unchanged by this work. ActionCart accepts the smaller
trial; Standalone's execution control verifies the safe fallback.
Recursive-call coverage permits a home only when no old value crosses the
call; it preserves Action's static-home semantics and adds no recursive
preservation summary.

Reproduce the gate comparison with:

```sh
python3 tools/compare6502/tn/leaf_facts.py \
  --output build/zp-home-lifetimes/tn-qualified --zp-home-lifetimes compare
```

Full dumps and binaries stay under ignored `build/`. The diagnostic generator
removes its temporary probe hook. Repository-wide compiler/VM suites were not
run for this MIR6502-only change.
