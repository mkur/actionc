# Acyclic call homes: focused qualification

2026-11-16. Completes the
[four-slice plan](../../MIR6502_ACYCLIC_CALL_HOMES_IMPLEMENTATION_PLAN.md).
Shared entry composition is in `d674c5c8`, residence verification in `4a87756a`,
and coordinated placement in `b18938d3`. The [receipt](qualification.json) binds
the measured compiler, inputs, configurations, artifacts and checks.

Optimized MIR6502 now enables source-home placement through supported acyclic
native calls. The conservative default stays disabled. Per-object lifetimes,
source ownership, candidate-specific contracts, final physical checks and strict
whole-image acceptance remain in force. There is one combined ordinary-call
trial, with no depth-specific trial or subset retry. Spill discovery and
terminal handoff retain their previous scope.

## Independent saving

The [control](../../../fixtures/runtime/acyclic_call_homes.act) has
`Hold -> Middle/Sibling -> Touch`, with real calls and inlining disabled. Both
middle routines already qualify through the old leaf-call placement. The new
scope lets Hold retain two words and a byte across them.

| Component | Gate off | Gate on |
| --- | ---: | ---: |
| Complete packaged image | 139 | 125 |
| Hold instructions | 76 | 67 |
| Hold inline homes | 5 | 0 |
| Middle instructions / inline homes | 18 / 0 | 18 / 0 |
| Sibling instructions / inline homes | 18 / 0 | 18 / 0 |

The **14-byte saving** consists of nine instruction bytes and five inline home
bytes. Hold uses `$E0/$E1` for `keep`, `$A3` for `n`, and `$E2/$E3` for `work`.
Middle and Sibling share `$A1` for `x` and `$A2` for `c`. The five additional
caller bytes avoid both callees' invocation footprints and proposed homes.

Changing Middle to native `=*` gives **145 → 131 bytes**, also a 14-byte saving.
Its own `x` and `c` remain inline and its body remains 22 instruction bytes.
Hold's `n` then uses `$A2`, sharing with Sibling's `c`: `n` survives Middle but
dies at the subsequent Sibling call. This exercises safe ancestor/descendant
sharing through the implemented value lifetimes.

Both comparisons use optimized ActionCart at `$3000` and differ only in
`enable_acyclic_call_homes`. Each side uses one ordinary-call trial, two
materializations, four emissions and eight inner emission probes. Named source
homes occupy seven distinct ZP bytes in the ordinary control and six in the
native-middle variant; these counts exclude transient scratch and anonymous
spills.

## TN result and remaining blockers

TN remains **9,811 packaged bytes**, byte-for-byte identical with the new gate
off/on and to the previous
[native-current-location baseline](../6502-native-current-location-callees/README.md).
All routine footprints and named home addresses are unchanged. There is no
baseline drift or newly selected TN home in this qualification.

| Control | Instruction bytes | Inline home bytes | Named ZP home bytes |
| --- | ---: | ---: | ---: |
| Ord | 51 | 0 | 5 |
| Items | 37 | 0 | 3 |
| FindItem | 53 | 0 | 3 |
| Strcpy | 20 | 0 | 3 |
| Instr | 55 | 0 | 3 |
| Fnamecmp | 105 | 0 | 4 |
| Next | 30 | 0 | 0 |
| Sort | 238 | 0 | 2 |
| PrintNonzeroDigit | 44 | 1 | 1 |

Sort also retains its anonymous `gap` spill at `$EC`. Across TN, mapped source
parameter/local homes total 24 bytes, sharing seven distinct ZP addresses;
scratch and anonymous spills are separate from that count.

The callers of Ord are **PopUp** and **Format**:

- PopUp's first gate-on rejection is now its call to Push. The composed entry
  proof retains an incoming carry requirement and fixed `$E8/$E9` inputs through
  ArenaCursor. It fails input supply before reaching Ord. PopUp also retains its
  separate Range/SArgs, address-exposure and entry-dependent-local restrictions.
- Format still fails the caller-body admission guard before call analysis. Its
  lowered body includes absolute/indexed address operations and computed stores;
  this extension adds no operation eligibility. It remains 263 instruction bytes
  and eight inline home bytes.
- Copy still fails its body guard and remains 673 instruction bytes plus 58
  inline bytes, of which 48 are arrays. Broader call depth does not remove that
  restriction.
- Alloc, Free and Push retain ArenaCursor entry obligations. CloseWindow and
  GoTo reach unsupported machine operations through MovePage and Relpos. The
  receipt includes the dependency and site diagnostics rather than labeling all
  these failures simply as nonleaf calls.

One further precision limit is deliberate: definite return-byte propagation
requires an existing scalar signature and actual definitions on every returning
path. Pointer-parameter routines such as Next lack that integer-only signature.
Their return-home dependencies stay conservative; this implementation does not
guess a result contract from a map signature or an incidental `$A0` store.

TN work counters match exactly: one ordinary-call trial, six materializations,
146 emissions, 876 inner probes and 17 preservation replays. One traced generation
sample took 20,094 ms with the gate off and 20,004 ms with it on. These are single
observations, not evidence of a speed improvement.

## Checks and retained artifacts

- Three new composed-entry groups cover diamonds, extra inputs, definite results,
  recursion, invalid later sites and transitive freshness. Existing entry/handoff
  and machine-liveness checks pass: 18 and 11 tests respectively.
- All 63 parameter-home tests pass, including coordinated proposals, source-order
  independence, exposed siblings, per-site lifetimes, real dead-boundary sharing
  and exact rollback when a new capture clobbers an unchanged caller.
- Sixteen ownership-certificate tests, five ABI unit tests, 11 call-ABI integration
  tests and 21 cross-call integration tests pass. One optional MADS test is ignored.
- The new VM group covers default and optimized ActionCart, an optimized
  Standalone control, and a native `=*` middle routine. LF and CRLF go through the
  actual file loader/parser. Seventy Hold invocations check 280 nested entries,
  host-derived results, arguments, boundary values, sentinels and stack balance.
- After enabling the optimized default, the two new default/gate groups and
  three historical leaf-scope integration groups pass. Existing lifetime-sharing
  and recursive-boundary VM groups also pass.

No full compiler suite, NIR sweep or unrelated backend suite was run. No frontend,
NIR or emission contract changed. The historical leaf-only negative controls
explicitly disable the new gate; new positive controls cover its expanded scope.

Large artifacts and the local measurement driver are retained under ignored
`build/acyclic-call-homes/`: control/native/TN gate-off/on images, maps,
relocations, selected MIR, event journals, configuration snapshots and logs.
Both A/B sides use the same compiled library. The receipt records the source
identity including the final optimized-default toggle.
