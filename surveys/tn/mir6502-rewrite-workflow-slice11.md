# MIR6502 rewrite workflow Slice 11 results

Date: 2026-07-21.

Scope: `samples/tn/modern/TN.ACT`, `--profile modern --backend mir6502`.
The migration baseline is commit `eeb20df`; the immediate binary comparison is
the Slice 10 output from commit `12061a3`.

## What changed

Post-home proof failures now retain a stable blocker category plus the routine,
block, operation index, and rewrite statistic. The fixed-point driver deduplicates
those sites within one invocation and reports aggregate counts through the
existing peephole report. This is diagnostic accounting only: the proof result
that blocks the rewrite remains authoritative.

Applied plans now accumulate estimated byte and cycle savings. Post-home
structural plans obtain those estimates from a small 6502-specific cost model
covering width and addressing mode. Selection still proceeds in this order:
legality, family priority, byte estimate, cycle estimate, window length, source
position. Cost cannot make an illegal plan eligible.

The reported totals are sums of local MIR plan estimates, not a prediction of
the final XEX delta. Several selection plans expose target operations rather
than immediately deleting final instructions, and later passes can combine or
supersede their effects. Exact size continues to come from the emitted listing
and XEX.

## TN result

The current aggregate report contains:

| Measurement | Count |
| --- | ---: |
| Applied-plan estimated bytes | 1,354 |
| Applied-plan estimated cycles | 1,616 |
| Blocked rewrite sites | 320 |
| Blocked: live home definition | 289 |
| Blocked: live register | 31 |

Blocked sites by rewrite family:

| Family | Blocked | Applied counter |
| --- | ---: | ---: |
| dead private scratch store | 270 | 23 |
| SSA-lite dead A load | 30 | reported as 4 dead register writes |
| spill store/reload pair | 12 | 59 |
| staged word-store forward | 4 | 1 |
| indirect load/spill consumer | 3 | 11 |
| direct increment/decrement | 1 | 2 |

All measured blockers are concrete liveness failures. No blocker was attributed
to unknown aliasing, an effects-analysis error, or an unsupported pointer pair.
Consequently there is no evidence for weakening alias/effect conservatism in
this slice. The large dead-scratch count instead identifies future candidates
for better staging or home allocation before the structural pass; deleting the
stores while their definitions are live would be incorrect.

Compared with the original `eeb20df` workflow baseline, the current XEX is 621
bytes smaller (12,719 to 12,098) and recognized instruction bytes fall from
11,715 to 11,094. The current listing has 4,952 recognized instructions, 1,400
`LDA`, 1,026 `STA`, 197 `JMP`, and 369 `JSR`. These are cumulative results of
Slices 1-11, not gains attributed solely to Slice 11.

Slice 11 itself does not change TN code selection:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| `TN-pre.mir` | 132,194 | `616449da61a62e5068d5c1549996b16dce48d69f178af2ce35f1f585e7bb565c` |
| `TN-materialized.mir` | 162,151 | `ef59cf1c7a1737685d02e966bfb5afb1c39d91abbad8786aef1bbe1894efbe61` |
| `TN.lst` | 158,764 | `a2769bcaf646a431330473633b28ed5ec7c9a4b3a62cca032fc8a6f3fffc3c8c` |
| `TN.xex` | 12,098 | `19de06ecd51e49c06edfda9ace7f6a3148f38b8dafabfce301426f6512a42ecd` |

The XEX hash exactly matches the saved Slice 10 artifact. The deterministic TN
compatibility/packaging check therefore supplements, and the identical binary
inherits, the prior runtime smoke result. There is still no automated screen
oracle.

## Compilation-time measurement

Five warm runs used the already-built debug `target/debug/actionc-emit` binary
to emit the TN load file. Wall-clock times were 3.70, 3.23, 3.23, 3.23, and
3.29 seconds; the median is 3.23 seconds (median user time 3.21 seconds). This
records the uncached-snapshot implementation before any caching work.

The report records 4,259 analysis builds across the deliberately fine-grained
groups and routine invocations. That makes generation-keyed snapshot reuse a
credible follow-up, but one TN debug timing is not enough reason to add cache
invalidation complexity to this slice. Benchmarking a broader corpus and
measuring per-analysis time should precede that work.

## Future MIRZ80 boundary

A MIRZ80 backend can reuse the target-independent graph, data-flow, and
dominance kernels in `src/analysis`, plus the workflow concepts of immutable
routine generations, transactional plans, deterministic overlap selection,
fixed-point scheduling, and stable proof blockers.

It must own its own CFG adapter, operation effects, locations, register/flag
liveness, ABI availability, phase verifier, and instruction cost model. The
current `PostHomeRewriteContext`, `MirHomeByte`, 6502 register sets, pointer
pairs, zero-page facts, and cost estimates remain in `mir6502`. None of these
facts is moved into NIR, preserving the target boundary required for a future
Z80 implementation.
