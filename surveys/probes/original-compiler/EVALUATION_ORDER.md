# Evaluation Order Probe Notes

These probes were added to distinguish original Action! semantics from
`actionc` extension behavior. The important result is that many tempting
side-effect tests are not valid original Action! source at all.

## Probe Set

- `eval_order_args.act` / `EVALARG.COM`
  - Intentionally original-invalid.
  - Original Action! rejects direct function calls as `PROC` arguments, e.g.
    `Take(F(),G(),H())`, with error 11.
  - `actionc` modern accepts this as an extension, so extension semantics must
    be defined by `actionc`; left-to-right evaluation is the safest rule.

- `eval_order_arith.act` / `EVALARI.COM`
  - Intentionally original-invalid.
  - Original Action! rejects `outB = FB() + GB()` because the raw result of
    `FB()` is still pending when `GB()` is reached. Targeted Action! 3.6 ROM
    probes report error 17. This is not a general ban on calls in arithmetic.
  - `actionc` compat enforces this pending-result rule; modern accepts the
    overlapping-call form as an extension. Both must preserve call order.

- `eval_order_compare.act` / `EVALCMP.COM`
  - Original Action! accepts function-call-vs-constant condition forms, including
    boolean and signed cases in this probe.
  - Original Action! rejects two function calls in one comparison, e.g.
    `IF FB() = GB() THEN ...`, with error 11.
  - `actionc` now accepts the boolean/signed tail, and also accepts calls on
    both sides of a conditional comparison as an extension in both profiles.

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

Original Action! permits multiple calls in one arithmetic expression when
each earlier return value is consumed before the next call. Calls, indexes,
and intermediate arithmetic therefore still need correct evaluation order.

Compat rejects overlapping raw arithmetic results and nested call arguments.
The separately supported two-call conditional comparison remains an extension.
Modern may accept further original-invalid forms, with left-to-right evaluation.

## Arithmetic result lifetime

The original source's `COMPILER.asm`, at `??expfunc`, checks `temps[0]` before
calling `pf.pf_` and reports an expression error when occupied. The call result
then occupies `args` (`$A0/$A1`). The same routine saves and restores occupied
ordinary temporaries at `$A2..$AF`; `AMPL.CGU.asm`'s `gettemps` excludes the
return area, and consuming an operand releases its temporary flag.

Action! 3.6 ROM probes confirm these distinctions (`F` returns a BYTE):

| Expression | Original result |
| --- | --- |
| `F(2)+F(3)` | Rejected |
| `(F(2))+(F(3))` | Rejected |
| `F(2)+(F(3)+1)` | Rejected |
| `F(2)+1*F(3)` | Rejected |
| `F(2)+1+F(3)` | Accepted, 6 |
| `F(2)+0+F(3)` | Accepted, 5 |
| `(F(2)*2-1)*(F(3)+1)` | Accepted, 12 |
| `(-F(2))+F(3)` | Accepted, 1 |
| `7-F(2)` | Accepted, 5 |
| `12/F(3)` | Accepted, 4 |
| `7 MOD F(3)` | Accepted, 1 |
| `8 RSH F(2)` | Accepted, 2 |
| `(F(2) LSH 0)+F(3)` | Rejected |
| `(F(2) LSH 1)+F(3)` | Accepted, 7 |
| `F(F(2)+1)` | Rejected |

A BYTE shift by zero is elided without consuming the result; a word shift
uses a helper, so `(W(256) LSH 0)+W(3)` is accepted and returns 259 for a CARD
function `W`. This source validation precedes optimization: folding an identity
operation must not change whether the source passes the compatibility check.

The original compiler also accepts the complete ANALOG #26 puLse source with
`xd(i)=(Rand(2)*2-1)*(Rand(3)+1)` unchanged. Both the ROM in `actionc-vm` and
`action-sandbox` produced the same 2,651-byte object in the investigation.
Regression tests in `src/codegen/tests/compat_calls.rs` cover acceptance,
rejection, runtime values, call counts, and call order under both runtimes.

Follow-up status:

- Done: unsigned function-return comparisons now avoid unnecessary `$AC`
  materialization and match `eval_order_cmpu.act` exactly.
- Done: original-accepted boolean/signed single-call IF conditions now compile
  and match `eval_order_compare.act` exactly.
