# Private aggregate call captures — slice 5

The same NIR snapshot-availability proof now reaches whole-value calls and
returns. Logical ABI operands remain ordinary complete `AggregateCapture`
locals of exactly the same nominal type and target layout. No verifier rule or
physical calling convention is relaxed.

## Consumption and ownership

For defined direct user callees, every substituted argument image must remain
unchanged until the call boundary. Shared aggregate ABI expansion copies all
inputs into distinct mutable parameter homes before any callee body operation.
Two arguments can therefore share one stable caller image while their mutable
callee values remain independent. Reentry after the entry copies does not
retroactively change consumed inputs. Calls still invalidate availability for
later reads, including on Atari's routine-static activation model.

The result buffer must be disjoint from each reused input image. Unknown,
external and indirect entry contracts remain staged. Evaluation of indirect
callees, left-to-right argument preparation and any required earlier argument
snapshot are unchanged. Effects, result operands and non-argument references
are separately inventoried; an eligible argument cannot hide another use.

At returns, an available whole capture may directly supply the return value.
The physical hidden-result-pointer copy still occurs. Globals, parameters and
inline subobjects cannot substitute for logical ABI captures. Complete union
tails, native padding and required variant validation are preserved.

Rewrites prefer whole-capture backing before eliminating it into a global or
field. This allows several ABI consumers to reuse one home instead of stranding
separate ABI copies. It is an eligibility ordering, not a target-specific cost
heuristic or an inliner.

## Fresh result routing

A separate publication proof recognizes a whole native automatic result buffer
with exactly one defined direct call producer and one copy consumer. The call
must establish its image on every incoming path to that copy, with no
intervening invalidating effects. The destination already passed the fresh,
single-initialization/unexposed-capture proof. The call can then write the final
home directly; the old result home and relay copy disappear.

This bounded producer rewrite rejects observable source identity, extra source
uses (including validation reads), routine-static activation and unknown entry
contracts. Shared lowering already directly initializes eligible native fresh
call bindings, including variants. Broader result routing through Atari pointer
relays and true callee hidden-result-slot forwarding remain explicit follow-ups,
not claims that all aggregate return/entry copies can disappear.

## Acceptance

Five compiler tests cover all four target layouts, all three aggregate kinds,
two shared arguments, returns, native result routing, argument barriers and
rejection of forged subobject ABI operands. Optimizer idempotence and physical
MIR copy reductions are checked: the two-argument probe loses two logical and
two ABI-expanded copy sites, while distinct callee entry copies remain.

Three VM tests compare classic/raw-NIR/optimized-NIR lanes and both runtimes:
mutable arguments and union tails, indirect target capture before mutating
arguments, and variant argument/return independence. Existing aggregate call,
indirect-call, identity and fresh-initialization regressions remain covered.

Only `aggregate_calls.optimized.nir` changes in this slice: `Updated` consumes
the existing stable `saved` whole home instead of an additional argument copy.
Its result and mutable parameter homes remain separate. Raw NIR and classic
emission are unchanged. Final reproducible costs and full-suite results belong
to slice 6; previous measurement files remain historical baselines.
