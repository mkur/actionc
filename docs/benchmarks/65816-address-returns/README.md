# Returned record addresses: final qualification

All four slices are implemented and qualified. Common MIR65816 placement owns
pure returned field/element address chains before allocating their intermediate
homes. A typed deferred Return evaluates one complete three-byte source into
A/X, preserves 24-bit carry/wrap and clears X.high. Original source-read identity,
final geometry, empty producer spans and complete selected/replay coverage remain
checked. Observable Loads retain their original capture; unsupported chains
retain their conservative strategies. The public ABI, NIR contracts and bank-zero
reservations are unchanged.

The frozen workload remains Exec816 `57df0d7`, 256 files and 1,156 routines.
The comparison baseline is completed call-flow stage 6. [Provenance](provenance.json)
binds compiler/runtime/fixture/tool inputs and all fifteen probe artifacts;
[baseline](baseline.json) preserves the original `Chain` MIR, bytes, homes and
source spans. Production address selection is commit `1b2b0e6a`; this final slice
adds observation and qualification. The pinned CLI and executed artifacts have
separate hash receipts.

| Profile | Prior code | Final code | Saving | Frame/spill/local-peak sum change |
| --- | ---: | ---: | ---: | --- |
| Optimized release | 425,954 | 425,868 | 86 | −24 each |
| Optimized guarded | 575,068 | 574,916 | 152 | −24 each |
| Raw guarded | 639,980 | 639,864 | 116 | −18 each |

No routine grows in code, frame, spill extent or local peak. Initialized data
remain 2,471 release bytes and 2,470 guarded bytes; zero-fill remains 277 bytes.
Added bank-zero reservation is **0 fixed bytes and 0 bytes per task**.
[Complete routine census](routines.csv.gz), [every changed routine](routine-changes.csv.gz)
and [static results](results.json) retain the measured quantities.

## Actual Chain output and admission

`MYDOSFILE.Chain` now uses **21 bytes and 12 instructions**, with zero frame,
spill extent and local peak in all three profiles. Its prior release size was
50 bytes/29 instructions; guarded profiles used 72 bytes/39 instructions.
The parameter is read once as a word plus one bank byte. There are no intermediate
DP/stack writes and no read of the pointed-to storage. The already empty
pointer-cast span is not counted as a separate instruction saving.
[Actual disassembly](Chain.asm) records each profile's final addresses.

| Profile | Broad typed address-return screen | Deferred Returns | Conservative Returns |
| --- | ---: | ---: | ---: |
| Optimized release | 12 | 4 | 8 |
| Optimized guarded | 12 | 4 | 8 |
| Raw guarded | 9 | 4 | 5 |

The screen follows returned values through typed cast/address definitions;
Loads and block parameters stop it. It is an observation cohort, not admission.
Each admission instead requires the successful replayed `return-address` request.
[Address-return census](address-returns.csv.gz) includes every screened site.

Optimized builds improve `FSLEASES.ObjectEntry`, `SDFS.Map`, `MYDOSFILE.Chain` and
`FSINFO.EnumerationState`. Raw builds admit both ObjectEntry branches, Map and
Chain. Their local-copy form of EnumerationState retains its existing path.
Refusals include dynamic indices, static bases, nonadjacent/cross-block chains
and an already-qualified borrowed cast. New ownership never displaces an earlier
qualified owner to force a larger cohort.

## Runtime and backend checks

The [validation receipt](validation.json) records 430 library tests, 88 integration
tests across fourteen targets and 389 native tests across 99 groups. The existing
one/four/six ignored tests retain their prior dispositions. Forty-six existing
observer checks pass, and publication is reproduced with `--check`.

Independent ca65 callers verify full A/X values, X.high, native modes, current-domain
D, DBR, both I states, S, canaries and exact three-byte source reads. Direct fields,
zero/nonzero literal elements and nested records match independent assembly;
zero-frame bodies perform no payload writes. Real-local and recursive frames,
fixed images, two o65 placements, low-word carry, bank-FF wrap, null/high-bit
pointers and dirty A/X/Y pass. Task/IRQ/NMI probes interrupt every reached
arithmetic, mode and teardown boundary in both tasks and reenter the same
functions in the IRQ domain. LF/CRLF inputs compile to identical images.

All **441 frozen native vectors** pass with identical values, arguments, external
access traces, cycles, private accesses and peaks. The rebuilt vector images
are byte-identical to stage 6. The new address patterns have independent fixtures;
these unchanged vectors do not measure an application runtime improvement.

| Profile | Cycles | Private accesses | Peak |
| --- | ---: | ---: | ---: |
| Optimized release | 71,833 | 20,655 | 33 |
| Optimized guarded | 76,021 | 20,655 | 33 |
| Raw guarded | 87,539 | 24,288 | 45 |

Compressed per-profile vector manifests/results and qualification receipts
retain their full oracles, pinned VM inputs and artifact hashes. No hardware
qualification is claimed.

Hosted results match stage 6: eight of fourteen pass. The six pre-existing
failures remain lists packaging's resident payload in bank zero (two modes)
and missing console includes in standalone DOS streams/routing (four).
Both guarded demo and OF816 boots pass with the measured compiler. Actual
unchecked packaging still reports `Hosted o65 requires a checked kernel`.
Build recipes preserve bank-zero budgets, task pools and runtime reservations.

| Hosted profile | Upper code + initialized data | Demo XEX | OF816 XEX |
| --- | ---: | ---: | ---: |
| Optimized guarded | 595,187 | 612,239 | 637,142 |
| Raw guarded | 660,135 | 678,321 | 703,224 |

Upper totals include linked platform assembly/data and exclude bank-zero loader,
resident code, boot storage and on-demand commands. Framing sizes are separate
quantities; guarded execution does not qualify an unchecked provider contract.

## Compiler cost and carried gates

Rust 1.99 settings match stage 6: optimization level 3, no debug information or
incremental compilation, and no CLI features. After builds and qualification,
one warm-up per profile precedes three serial rounds with alternating order.
All nine wait4 samples and every pinned image check are retained in
[host results](host-results.json). [Host comparison](host-comparison.json) binds
baseline lineage and verifies the 5% time/10% RSS limits for every median.

| Profile | Prior wall (s) | Final wall (s) | Change | Prior RSS (MiB) | Final RSS (MiB) | Change |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Optimized release | 56.048 | 58.201 | +3.84% | 1,216.6 | 1,000.3 | −17.78% |
| Optimized guarded | 41.518 | 43.502 | +4.78% | 1,326.2 | 1,044.4 | −21.25% |
| Raw guarded | 45.739 | 47.866 | +4.65% | 1,233.7 | 994.3 | −19.40% |

These are observed costs on this host, not an attribution of the RSS change to
four address Returns. Background compiler activity from another checkout was
observed during sampling; this limits interpretation of the timing differences.
All medians satisfy the retained review limits.

[Final acceptance](final-acceptance.json) preserves the original targets.
Release code remains **3,168 bytes** above 422,700; representative code remains
**415 bytes** above 9,235; native private traffic remains **90 accesses** above
20,565. The unchecked hosted-provider and separate 256 KiB application objectives
also remain open. Prior subsystem, loop/call, cycle and peak benefits remain
qualified. Completing this plan does not reset those gates.

## Reproduction

Restore the immutable inputs from the [call-flow stage-0 scorecard](../65816-call-flow-stage0/README.md)
and retain stage 6 as the reference. Use a fresh output directory for collection.
Build and pin `actionc-65816` with the settings above. The
[implementation plan](../../MIR65816_ADDRESS_RETURN_PLAN.md#validation-entry-points)
lists the scoped backend/native commands; run the seven hosted cases in both
modes using `exec_record_hosted.py`. Retain existing failure records.

```sh
python3 -B tools/compare65816/exec_call_candidate.py collect \
  --output target/address-return-final --stage 3
python3 -B tools/compare65816/exec_record_vectors.py \
  --base target/record-placement-stage0 \
  --output target/address-return-final/native-vectors \
  --binary target/address-return-final/actionc-65816
python3 -B tools/compare65816/exec_record_qualification.py build \
  --base target/record-placement-stage0 \
  --output target/address-return-final/hosted-profiles --compiler-root . \
  --binary target/address-return-final/actionc-65816
```

For each vector profile set `A816_COMPARISON_MANIFEST` and
`A816_COMPARISON_RESULTS` to absolute candidate paths, then run
`qualify.py --test code_quality -j2 -- --ignored`. Keep successful root/native
suite logs under the candidate prefix. Finish all builds and qualification
before the serial measurement and final publication:

```sh
python3 -B tools/compare65816/exec_call_measure.py \
  --base target/record-placement-stage0 --output target/address-return-final \
  --binary target/address-return-final/actionc-65816
python3 -B tools/compare65816/exec_address_return_report.py \
  --output target/address-return-final --reference target/call-flow-stage6 \
  --destination docs/benchmarks/65816-address-returns --qualify --check
```

Omit `--check` when publishing a fresh generation. The report authenticates
artifacts, source generations, previous call/output facts, routine resources,
vector oracles, hosted dispositions and compiler-cost limits. The final
[evidence hashes](qualification-evidence-sha256.json) bind the published files.
