# Evaluation Order Probe Notes

These probes were added to distinguish original Action! semantics from
`actionc` extension behavior. The important result is that many tempting
side-effect tests are not valid original Action! source at all.

## Probe Set

- `eval_order_args.act` / `EVALARG.COM`
  - Intentionally original-invalid.
  - Original Action! rejects direct function calls as `PROC` arguments, e.g.
    `Take(F(),G(),H())`, with error 11.
  - `actionc` currently accepts this as an extension, so extension semantics must
    be defined by `actionc`; left-to-right evaluation is the safest rule.

- `eval_order_arith.act` / `EVALARI.COM`
  - Intentionally original-invalid.
  - Original Action! rejects function-call operands in compound arithmetic, e.g.
    `outB = FB() + GB()`, with error 11.
  - `actionc` currently accepts this as an extension. Any optimizer/codegen path
    for these expressions should preserve left-to-right evaluation.

- `eval_order_compare.act` / `EVALCMP.COM`
  - Original Action! accepts function-call-vs-constant condition forms, including
    boolean and signed cases in this probe.
  - Original Action! rejects two function calls in one comparison, e.g.
    `IF FB() = GB() THEN ...`, with error 11.
  - Current `actionc` rejects part of the broad original-accepted probe with
    `codegen only supports scalar IF conditions and unsigned comparisons`; this
    covers the boolean/signed tail and should be treated as a real coverage gap.

- `eval_order_cmpu.act` / `EVALCMPU.COM`
  - Narrow unsigned function-call-vs-constant comparison subset.
  - Original and `actionc` both compile it.
  - `actionc` preserves the single call evaluation point, but differs in code
    shape starting at `<`: original compares the return value in `$A0` directly,
    while `actionc` stores `$A0` to `$AC` and reloads before `CMP`.

- `eval_order_index.act` / `EVALIDX.COM`
  - Original Action! accepts function-call indexes for reads and constant stores,
    e.g. `outB = ba(FB())`, `ba(FB()) = 33`, `outW = ca(FB())`, and
    `ca(FB()) = $3333`.
  - Original Action! rejects multiple calls in one indexed assignment, e.g.
    `ba(FB()) = GB()`, with error 11.
  - Current `actionc` compat output is byte-exact against original for the
    supported index forms.

## Conclusions

For original-compatible source, Action! avoids most ambiguous side-effecting
binary evaluation cases by rejecting them. The compiler still needs correct
order for accepted function-call indexes and single-call condition forms; the
current supported index paths are exact.

Compat policy: these original-invalid forms are rejected by the `compat`
profile. `modern` may keep them as extensions, but any retained extension should
use strict left-to-right evaluation as its semantic rule.

Follow-up status:

- Done: unsigned function-return comparisons now avoid unnecessary `$AC`
  materialization and match `eval_order_cmpu.act` exactly.
- Done: original-accepted boolean/signed single-call IF conditions now compile
  and match `eval_order_compare.act` exactly.
