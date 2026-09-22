# Native 65816 remaining edge-copy inventory

Status: measured against main `c79697b` and the qualified
[control-flow 3c baseline](benchmarks/65816-control-flow/3c/after/tables.md)
on 2026-09-22. This is a read-only inventory for roadmap slice 4. Compiler
selection, allocation, ABI and stack guards have not changed.

This inventory is frozen at `94421a3`; use that historical checkout to reproduce
its exporter/hash checks. The acyclic portion is now implemented and separately
[qualified](MIR65816_ACYCLIC_EDGES.md). The selective-staging column below remains
a forecast for the broader optimization, including cyclic edges.

## Scope and evidence

The 14-kernel / 66-vector comparison corpus supplies 28 raw/optimized Action
builds and 132 Action measurement records, each already executed with both
incoming I states in debug and release hosts. Those qualified execution counts
are reused; this task does not claim a fresh VM qualification run. All 264
Action/vbcc records and the known optimized vbcc `unlink` failure are retained.

The [typed exporter](../tests/mir65816_copy_inventory.rs) rebuilds every Action
image and requires exact equality with its measured serialized artifact. It
enumerates every counted routine's MIR transfers, checks logical source/target
identity, and exports source/destination/staging byte ranges. Main drivers outside
the benchmark's counted ranges are excluded. The
[inventory checker](../tools/compare65816/inventory_copies.py) verifies source and
artifact hashes, exact copy encodings and targets, and per-instruction execution
counts. All existing word/direct-copy counts must be accounted for, including
unreached sites. Empty fallthroughs have no invented instruction count.

Saved [facts](benchmarks/65816-copy-inventory/facts.json) and
[inventory](benchmarks/65816-copy-inventory/inventory.json) contain every edge,
physical range, dependency, per-vector count, staging reservation and forecast.
The checker rejects unknown source encodings instead of silently omitting them.

## Measured copies

Counts below are static sites or executions per incoming I state across the
whole corpus; debug and release agree and are not added together.

| Edge kind | Raw sites | Optimized sites | Copy-edge executions |
| --- | ---: | ---: | ---: |
| Empty | 20 | 14 | Not counted as copies |
| Already direct single-word | 0 | 4 | 92 |
| Staged, acyclic in existing order | 0 | 1 | 6 |
| Staged, containing a cycle | 0 | 1 | 48 |
| Mixed-width / byte fallback | 0 | 0 | 0 |

There are ten static assignments and 254 assignment executions: four direct
assignments execute 92 times, while six staged assignments execute 162 times.
No self-copies, repeated sources, partial source/destination overlaps or
acyclic edges requiring reordering occur in this corpus. These are coverage
limits, not claims that such MIR shapes cannot occur. Raw code still contains
ordinary stack traffic and moves; it simply has no nonempty MIR edge assignments
in these samples.

All remaining staging occurs in optimized `loop_rotation`. Every range below
is relative to the established body S; each source/destination word occupies
the named byte and the next byte. An arrow means source to destination.

| Edge / first copy PC | Assignments in current order | Staging word starts | Executions per vector |
| --- | --- | --- | ---: |
| Initialization, `$010038` | `$06 → $0A`; `$08 → $06`; `0 → $08` | `$0E`, `$12`, `$16` | 1 |
| Backedge, `$010074` | `$06 → $0A`; `$0A → $06`; `$0C → $08` | `$0E`, `$12`, `$16` | 8 |

The initialization forms a safe chain in its existing order: each old source
is consumed before a later destination overwrites it. The backedge exchanges
the words at `$06` and `$0A`, and independently copies the counter from `$0C`
to `$08`. Six input vectors reach both sites, giving 6 and 48 edge executions.

The other nonempty edges are already direct: optimized `sum_loop` initialization
and backedge execute 5 and 53 times, and optimized `byte_sum` initialization and
backedge execute 5 and 29 times. Their savings must not be counted again.

## Candidate comparison

These are conditional instruction/traffic forecasts, not measurements of a
changed compiler. Both candidates retain every destination assignment, all
staging reservations, the current frame extent, block order and transfer forms.
Each avoided staging store/reload pair saves four code bytes, two instructions,
ten cycles, two stack-byte reads and two stack-byte writes per execution.

| Candidate | Static staging pairs avoided | Avoided pair executions | Corpus cycles saved |
| --- | ---: | ---: | ---: |
| Direct whole acyclic edges, existing order | 3 | 18 | 180 |
| Selective source staging, existing destination order | 5 | 114 | 1,140 |

For selective staging, capture a source early only if an earlier destination
assignment would overwrite any of its bytes. Emit all destination assignments
in the original order, loading other sources directly at that point. For the
backedge, save old `$0A` before writing `$0A`; the `$06 → $0A` and `$0C → $08`
assignments need no staging. This handles the observed swap without reordering
destinations or introducing X/Y/DP residency. It also retains the final value
loaded into A and its N/Z result. Calls and helpers cannot occur inside these
verified straight-line copy sequences; sources are private stack homes or
immediates, with staging disjoint from every source and destination.

| Optimized `loop_rotation(13)` | Measured baseline | Whole-edge forecast | Selective-staging forecast |
| --- | ---: | ---: | ---: |
| Code bytes | 160 | 148 | 140 |
| Cycles | 1,146 | 1,116 | 956 |
| Instructions | 268 | 262 | 230 |
| Stack-byte reads | 181 | 175 | 143 |
| Stack-byte writes | 178 | 172 | 140 |
| Peak stack bytes | 26 | 26 | 26 |

All six rotation vectors have the same baseline counts and forecasts. Both
candidates predict no change for other corpus builds, including `sum_loop`.
The JSON records per-edge/per-vector forecasts for each candidate; byte savings
are static and must not be summed once per vector.

## Staging reservations

| Optimized routine | Reserved staging bytes | Bytes written by edge copies | Bytes never written by edge copies |
| --- | ---: | ---: | ---: |
| `loop_rotation` | 12 | 6 | 6 |
| `sum_loop` | 4 | 0 | 4 |
| `byte_sum` | 4 | 0 | 4 |

All other counted routines reserve no edge staging. The six unused rotation
bytes are the upper halves of three four-byte slots. The other eight bytes
belong to wholly unused slots retained after direct single-word emission.
These 14 bytes are an allocation finding, not a promised frame-size reduction:
alignment, incoming argument displacements, frame extent, guards and interrupt
reserves must be revalidated in a separate reservation-shrinking slice.

## Recommended implementation slice

Prefer **selective source staging for all-word stack edges, retaining destination
order**, over a whole-acyclic-edge-only optimization. It addresses the hot cycle
and has a small dependency rule: retain staging exactly where earlier writes
would destroy a source. Keep all assignments, frame reservations and the current
fallback for mixed widths, unsupported operands or unproved overlap. Self-copy
elimination, home coalescing and frame shrinking remain separate work.

Before implementation, extend focused cases for self-copies, repeated sources,
longer cycles, incoming/mutable parameter homes, partial overlaps and both branch
arms. Qualification must check exact destination values, final A/N/Z, surviving
memory order, alias/call boundaries, relocated o65 and IRQ/NMI interruption at
the changed copy instructions. The inventory's abstract byte-memory model checks
the dependency rule on 216 small graphs, but it is not CPU qualification.

## Reproduction and checks

With the immutable `target/control-3c-after` artifacts available:

```sh
A816_COMPARISON_MANIFEST="$PWD/target/control-3c-after/manifest.json" \
  A816_COPY_INVENTORY_FACTS="$PWD/target/copy-inventory-facts.json" \
  cargo test --test mir65816_copy_inventory -- --ignored
python3 -B tools/compare65816/inventory_copies.py target/control-3c-after \
  --facts target/copy-inventory-facts.json \
  --output docs/benchmarks/65816-copy-inventory/inventory.json --check
python3 -B -m unittest discover -s tools/compare65816 -p 'test_*.py'
```

Validation: the exporter passes for all 28 builds, with additional LF and CRLF
source parses producing identical serialized images; all 31 Python comparison
tests pass, including seven new inventory tests. Tests cover dependency direction,
cycles, self-copies, repeated sources, partial overlap, exact byte/target checks,
corruption rejection, byte fallback and LF/CRLF listing parsing. No compiler or
native VM implementation changed, so unrelated suites were not rerun.
