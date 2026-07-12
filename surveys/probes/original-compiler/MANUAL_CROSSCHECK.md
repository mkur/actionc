# Manual Cross-Check

Source: `../code/JAC/doc/action-manual.pdf`

The PDF is image-based. It was rendered with Ghostscript and OCRed with
Tesseract for this pass; section references below are from the manual text, not
from the generated OCR line numbers.

## Findings Confirmed By The Manual

- Code origin: Part VII, Chapter 5 confirms that a fixed compile address should
  set both APPMHI/CODE (`$0E`) and CODEBASE (`$491`) before compilation. This
  matches the probe convention:

```action
SET $E=$3000
SET $491=$3000
```

- Fixed-address machine-language calls: section 9.4 confirms the byte-level
  parameter ABI for machine-language routines:

```text
A register  = 1st parameter byte
X register  = 2nd parameter byte
Y register  = 3rd parameter byte
$A3         = 4th parameter byte
...
$AF         = 16th parameter byte
```

  This matches the `ARGTHR`, `SARGS`, `STRPASS`, and `RECORDS` probe
  observations for caller-side byte order. The manual describes external
  machine-language calls; the `SArgs` callee prologue shape remains a probe
  finding.

- Functions: Chapter 6 confirms that functions return only fundamental numeric
  types (`BYTE`, `CARD`, `INT`) and use `RETURN(<arith exp>)`. Procedures cannot
  return a value. Multiple `RETURN`s are legal in both procedures and functions.

- Parameters: Chapter 6 confirms up to 8 declared parameters. A call may pass
  fewer parameters than declared, but not more. The manual also confirms that
  the compiler coerces parameter types rather than rejecting mismatches.

- Array/pointer/record names as parameters: Chapter 6 says array, pointer, and
  record names used as parameters are treated as pointers to the first element,
  the value, or the first field respectively. This agrees with array-name and
  record-value passing probes.

- Pointers: Chapter 8 confirms that pointer variables contain addresses and are
  stored as two bytes.

- Arrays: Chapter 8 confirms arrays are contiguous cells, index 0 is the first
  element, and element cell size is one byte for `BYTE`, two bytes for `CARD`
  and `INT`. It also confirms that an array name is pointer-like.

- Strings: Chapter 3 and Chapter 8 confirm that string constants are stored
  with a leading length byte. Chapter 8 explicitly notes the first byte of a
  string constant is the length and is element 0 of the array containing the
  string. This agrees with `STRINIT`, `STRLIT`, `STRPASS`, `STRMUT`, and
  `STRLOC`.

- Records: Chapter 8 and the grammar appendix confirm `TYPE` fields are
  fundamental variable declarations. Pointer fields and array fields are not
  part of normal `TYPE` records. This explains the rejected probe variant:

```action
TYPE Pair=[BYTE tag CARD word CHAR POINTER ptr]
```

  The manual also says arrays of records and array fields in records are not
  directly supported; it suggests "virtual records" over `BYTE ARRAY` storage
  as the workaround.

## Manual Errata That Matter

- Record pointer arguments other than the first are a documented cartridge bug:
  a declaration such as `PROC Test(BYTE x, REC POINTER p)` may produce error 7.
  The errata suggests omitting the comma/newline-splitting as a workaround. We
  should decide whether `actionc` wants to accept the intended syntax or emulate
  this original compiler bug.

- TYPE fields used directly as routine parameters are documented as generating
  incorrect code in some cartridge versions. The suggested workaround is to copy
  the fields to temporaries before passing them.

- CARD fields in TYPE records are documented as generating incorrect code in
  versions 3.2 through 3.4. Our `RECORDS.COM` result appears consistent with a
  newer/fixed compiler, but this is worth remembering if probe outputs come from
  a different cartridge version later.

- Hexadecimal array dimensions are documented as generating incorrect code in
  all versions; decimal dimensions are the recommended workaround.

- `ELSEIF a(i)=...` where `a` is an array and `i` is `CARD`/`INT`, and
  `ELSEIF p^=...` where `p` is a pointer, are documented as incorrect-code
  cases.

- Complex relational expressions in `UNTIL` are documented as incorrect-code
  cases in some versions.

- `actionc` currently takes a sane-default position on these documented
  incorrect-code cases: supported forms compile according to the language intent
  instead of reproducing the cartridge bug, while unsupported complex `UNTIL`
  forms are rejected rather than emitted as suspect code. Bug-for-bug behavior
  should stay behind a future explicit compatibility mode if we ever need it.

- `*`, `/`, and `MOD` have an implied `INT` result according to the errata.
  This matches the runtime-helper focus on signed arithmetic.

- Fixed-address `PROC` declarations for machine-code routines have a documented
  code-generation caveat in some versions. The errata suggests adding an empty
  code block after the declaration:

```action
PROC CIO=$E456() []
```

## Remaining Probe Candidates

These are the only probes that still look useful before broad implementation:

1. Optional parameters / omitted arguments
   - Rationale: the manual says calls may pass fewer parameters than declared.
   - actionc coverage: calls now accept omitted trailing arguments and emit
     only the supplied argument bytes.
   - Remaining probe value: original `.COM` comparison would still be useful to
     confirm whether omitted parameter bytes are merely left as current ABI
     contents, as `actionc` assumes.

2. Array/string assignment to an unsized `BYTE ARRAY`
   - Rationale: the manual says assigning a string constant to an array name
     allocates new string storage and changes the array pointer, rather than
     copying into old storage.
   - Probe: `BYTE ARRAY s; s="ONE"; s="TWO";` plus reads of the pointer and
     string bytes.

3. External fixed-address calls
   - Rationale: `abi_system_call.act` exists but has no original `.COM` yet,
     and the manual/errata call out fixed-address routine behavior.
   - actionc coverage: fixed-address `PROC Sys=$1234(...)` and the `[]` body
     marker form are now covered by codegen tests.
   - Remaining probe value: original `.COM` comparison would still be useful
     for byte-for-byte confirmation.

4. Record pointer parameter after another parameter
   - Rationale: the manual errata says this is an original compiler bug.
   - Probe: try `PROC Test(BYTE x, REC POINTER p)` and the documented
     workaround form. This is only needed if we want to mimic original syntax
     failures closely.

5. TYPE fields as call arguments
   - Rationale: documented original incorrect-code case.
   - Probe: pass `rec.field` directly to a procedure and compare with the
     temporary-variable workaround. This is only needed if we plan bug-for-bug
     compatibility.
