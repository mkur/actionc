# Next Probe Plan

This plan uses `action-compiler-vm/scripts/run-probe` to generate original
compiler `.COM` files repeatably. Keep each probe narrow: one codegen question,
one small object file, and a short note about the expected comparison point.

Generated VM originals should go to:

```text
outputs/vm/
```

Then compare against `outputs/actionc/` listings/load files and promote stable
findings into tests.

## 1. Optional Arguments

Probe name: `optional_args.act`

Question:

- When a call supplies fewer arguments than declared, what exact bytes does the
  original compiler emit?
- Are omitted trailing argument bytes untouched, zeroed, or still part of the
  callee prologue layout?

Rationale:

- The manual allows fewer arguments than declared.
- `actionc` currently emits only supplied argument bytes and leaves omitted ABI
  locations unchanged.
- A narrow original comparison can validate that sane default or expose a
  prologue detail we missed.

Suggested source shape:

```action
SET $E=$3000
SET $491=$3000

BYTE seen1 seen2 seen3

PROC Take(BYTE a,b,c)
  seen1=a
  seen2=b
  seen3=c
RETURN

PROC Main()
  Take(1)
  Take(2,3)
RETURN
```

Implementation value:

- If the generated prologue/call sites match the current model, turn it into a
  regression test for optional arguments.

## 2. Unsized Array/String Pointer Assignment

Probe name: `array_assign.act`

Question:

- Does assigning a string literal to an unsized `BYTE ARRAY`/`STRING` update the
  stored pointer to new length-prefixed storage?
- How does the original lay out multiple assigned literals in one routine?

Rationale:

- The manual says string assignment to an array name changes the pointer rather
  than copying into prior storage.
- `actionc` implements this now, but the exact placement of inline literal
  storage is useful for compatibility comparisons.

Suggested source shape:

```action
SET $E=$3000
SET $491=$3000

BYTE ARRAY s
BYTE len ch

PROC Main()
  s="ONE"
  len=s(0)
  ch=s(1)
  s="TWO"
  len=s(0)
  ch=s(1)
RETURN
```

Implementation value:

- Confirms pointer assignment and inline literal layout.
- Helps decide whether literal pooling/jump-over placement matters for
  comparison.

## 3. Fixed-Address External Calls

Probe name: `external_call.act`

Question:

- What does the original compiler emit for `PROC Name=$addr(...)` calls?
- Does the empty body marker `[]` affect emitted bytes?
- Does the caller-side byte ABI match user routine calls for mixed `BYTE`,
  `CARD`, pointer, and omitted arguments?

Rationale:

- The manual documents fixed-address machine-language calls and mentions
  cartridge-version caveats.
- `abi_system_call.act` exists but does not yet have a captured original `.COM`.
- `actionc` supports fixed-address declarations now; this is the direct
  compatibility check.

Suggested source shape:

```action
SET $E=$3000
SET $491=$3000

PROC Sys=$1234(BYTE a CARD w BYTE POINTER p) []

BYTE b
CARD c
BYTE ARRAY data=[1 2 3]

PROC Main()
  b=$11
  c=$2233
  Sys(b,c,data)
  Sys($44)
RETURN
```

Implementation value:

- Validates absolute `JSR` lowering and argument-byte staging.
- Cross-checks optional arguments on fixed-address calls.

## 4. Record Field Arguments

Probe name: `record_args.act`

Question:

- What does the original compiler emit when `rec.field` is passed directly as a
  routine argument?
- Does the documented incorrect-code warning appear with this cartridge?

Rationale:

- The manual errata warns that `TYPE` fields used as call arguments may produce
  bad code in some versions.
- `actionc` currently takes the sane-default route and supports intended
  behavior.
- This probe tells us whether our current cartridge behaves correctly or is
  bug-compatible with the errata.

Suggested source shape:

```action
SET $E=$3000
SET $491=$3000

TYPE Pair=[BYTE tag CARD word]
Pair rec
BYTE out
CARD wide

PROC Take(BYTE b CARD c)
  out=b
  wide=c
RETURN

PROC Main()
  rec.tag=$12
  rec.word=$3456
  Take(rec.tag,rec.word)
RETURN
```

Implementation value:

- If original output is sane, keep `actionc` behavior.
- If original output is wrong, document as a known divergence unless strict
  bug compatibility becomes a goal.

## 5. Record Pointer Parameter Position

Probe name: `record_ptr_order.act`

Question:

- Does this cartridge reject or miscompile a record pointer parameter that is
  not the first parameter?
- Does the documented workaround form change parsing or output?

Rationale:

- The manual errata calls out declarations like
  `PROC Test(BYTE x, REC POINTER p)` as problematic on some versions.
- `actionc` accepts the intended syntax.
- This should be probed only to classify compatibility; it is not a reason to
  regress `actionc` by default.

Suggested source shape:

```action
SET $E=$3000
SET $491=$3000

TYPE Pair=[BYTE tag CARD word]
Pair rec
BYTE got

PROC Touch(BYTE x, Pair POINTER p)
  got=x+p.tag
RETURN

PROC Main()
  rec.tag=5
  Touch(7,rec)
RETURN
```

Implementation value:

- Documents whether this cartridge has the errata behavior.
- Helps decide whether a future strict-compatibility mode needs parser/codegen
  switches.

## 6. Dynamic Index Scaling Edges

Probe name: `index_scaling.act`

Question:

- How does the original lower dynamic indexes for `BYTE`, `CARD`, and `INT`
  arrays, especially when the index expression is not a simple variable?

Rationale:

- Dynamic indexing is central to arrays, strings, and pointer-backed data.
- We have broad array probes, but a small scaling-focused probe gives clearer
  diffs for codegen improvements.

Suggested source shape:

```action
SET $E=$3000
SET $491=$3000

BYTE i,b
CARD c
BYTE ARRAY ba=[1 2 3 4]
CARD ARRAY ca=[10 20 30 40]
INT ARRAY ia=[-1 -2 -3 -4]

PROC Main()
  i=1
  b=ba(i+1)
  c=ca(i+1)
  c=ia(i+1)
RETURN
```

Implementation value:

- Confirms byte vs word index scaling.
- Highlights whether complex index expressions need temporary storage or can be
  lowered directly.

## 7. Boolean/Relational Control Edges

Probe name: `bool_edges.act`

Question:

- What branch shapes does the original emit for signed relational conditions in
  `IF`, `WHILE`, and simple `UNTIL`?
- Which complex `UNTIL` forms are accepted, rejected, or miscompiled?

Rationale:

- Signed comparisons and control flow are high-value compiler compatibility
  areas.
- The manual warns about complex `UNTIL` expressions; `actionc` currently
  rejects unsupported suspect forms instead of emitting questionable code.

Suggested source shape:

```action
SET $E=$3000
SET $491=$3000

INT a,b
BYTE out

PROC Main()
  a=-1
  b=1
  IF a<b THEN
    out=1
  FI
  WHILE a<b
  DO
    a==+1
  OD
  DO
    b==-1
  UNTIL b=0
RETURN
```

Implementation value:

- Good target after current data-form work, before deeper optimizer/codegen
  tweaks.
- Helps classify any remaining control-flow divergences as bugs vs deliberate
  sane defaults.

## Recommended Order

1. `optional_args.act`
2. `array_assign.act`
3. `external_call.act`
4. `index_scaling.act`
5. `bool_edges.act`
6. `record_args.act`
7. `record_ptr_order.act`

The first five are likely to produce implementation-relevant fixes. The last
two are mostly compatibility classification for documented original compiler
bugs.

## Capture Status

Implemented and captured with `action-compiler-vm/scripts/run-probe`:

- `optional_args.act` -> `outputs/vm/OPTARGS.COM`
- `array_assign.act` -> `outputs/vm/ARRASN.COM`
- `external_call.act` -> `outputs/vm/EXTCALL.COM`
- `index_scaling.act` -> `outputs/vm/IDXSCALE.COM`
- `bool_edges.act` -> `outputs/vm/BOOLEDGE.COM`
- `record_args.act` -> `outputs/vm/RECARGS.COM`
- `array_inline_global_threshold.act` -> `outputs/vm/ARRTHG.COM`
- `array_inline_local_threshold.act` -> `outputs/vm/ARRTHL.COM`
- `array_inline_boundary.act` -> `outputs/vm/ARRTHB.COM`

Expected original compiler failure:

- `record_ptr_order.act` fails with error 7, matching the manual errata for
  record pointer parameters that are not first.

Current `actionc` comparison:

- `optional_args`, `array_assign`, `external_call`, `bool_edges`,
  `record_args`, and `record_ptr_order` compile under `actionc`.
- `index_scaling` now compiles under `actionc`; outputs are in
  `outputs/actionc/index_scaling.*`.
- Successful `actionc` load files currently differ byte-for-byte from the VM
  originals; use the listings and load-file diffs as implementation work items.

Threshold classification:

- `array_inline_global_threshold.act` and `array_inline_local_threshold.act`
  probe `BYTE ARRAY(n)` storage layout in 128-byte increments.
- `array_inline_boundary.act` pins the exact boundary: both global and local
  sized `BYTE ARRAY` declarations are inline through 256 bytes and switch to
  descriptor/backing storage at 257 bytes.
- `array_inline_threshold.act` is a broader combined stress version. It captures
  useful local snapshots but is too large for reliable VM save-output use, so
  prefer the split probes above for repeatable evidence.
- Sweep status: `array_inline_local_threshold.act` is exact. The boundary and
  global-threshold probes are accepted divergences: the boundary probe only
  differs in original local inline metadata residue, and the global probe only
  differs in descriptor pointer words for unsaved backing ranges.
