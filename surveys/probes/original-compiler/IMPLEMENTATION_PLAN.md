# Implementation Plan

This plan is based on the original compiler probes plus the official manual
cross-check. Keep changes small: one behavior family per patch, with focused
tests and probe comparisons.

## Guiding Rules

- Treat `ABI.md` and probe notes as the codegen compatibility contract.
- Treat `MANUAL_CROSSCHECK.md` as the language-level validity reference.
- Prefer implementing useful compatible behavior over emulating documented
  original compiler bugs, unless a strict compatibility mode is added later.
- Keep each implementation slice independently testable.

## Steps

1. Stabilize probe docs as the reference.
   - Freeze the current ABI and manual notes as the working contract.
   - Use original `.COM` probes to resolve codegen details.

2. Add semantic model support for string aliases.
   - Ensure `DEFINE STRING="CHAR ARRAY"` flows cleanly through semantic
     analysis.
   - Treat `STRING` as `CHAR ARRAY`, not as a distinct runtime type.

3. Implement global initialized string storage.
   - Support `BYTE ARRAY s="ABC"`, `STRING s(0)="ABC"`, and
     `CHAR ARRAY s(3)="ABC"`.
   - Emit length-prefixed storage: `$03 "ABC"`.
   - Preserve index 0 as the length byte.

4. Implement constant indexed string/`CHAR ARRAY` reads and writes.
   - Start with global storage only.
   - Lower constant indexes like byte-array absolute access: `base + index`.

5. Implement local initialized string/`CHAR ARRAY` storage.
   - Match original layout: fixed load-segment storage before the owning
     routine trampoline.
   - Do not model it as stack storage.

6. Implement string and `CHAR ARRAY` parameter reads/writes.
   - Use pointer-backed access through `$AE/$AF` and `($AE),Y`.
   - Support constant and dynamic byte indexes.

7. Implement passing named arrays/strings to routines.
   - Passing `hello` or `raw` should pass the base pointer in `A/X`.
   - If total argument bytes exceed 2, use the existing compatible `SArgs`
     path or deliberately document any temporary direct-prologue gap.

8. Implement string literal arguments.
   - Support calls such as `Take("HI")`.
   - Emit inline length-prefixed literal storage in the caller body.
   - Jump over the literal and pass the address of its length byte.
   - Defer literal pooling.

9. Implement unsized `BYTE ARRAY` pointer assignment.
   - Support pointer-style assignment first, such as `s = other`.
   - Then support string literal assignment, such as `s = "ONE"`.
   - Assignment from a string literal should allocate/emplace new
     length-prefixed storage and update the array pointer, not copy into old
     storage.

10. Implement basic record storage.
    - Support packed fundamental fields only.
    - Example: `TYPE Pair=[BYTE tag CARD word]` and `Pair rec`.
    - Reject or defer pointer fields, array fields, and arrays of records.

11. Implement direct record field access.
    - Support `rec.tag = $11` and `x = rec.word`.
    - Use packed offsets and little-endian storage.

12. Implement record pointer field access.
    - Support `PROC Touch(Pair POINTER rp)` with `rp.tag` and `rp.word`.
    - Compute `base + field_offset` through `$AE/$AF`, then use `($AE),Y`.

13. Implement passing record values to record pointer parameters.
    - `Touch(rec)` should pass the base address of `rec` in `A/X`.

14. Add external fixed-address call support.
    - Support declarations like `PROC Sys=$1234(BYTE a CARD w)`.
    - Use the same caller byte ABI.
    - Probe `abi_system_call.act` before finalizing edge behavior.

15. Add optional/omitted argument behavior.
    - The manual says fewer arguments than declared are allowed.
    - Probe first, then implement the confirmed parameter-storage behavior.
    - Sane default: allow omitted trailing arguments, emit only supplied
      argument bytes, and do not synthesize zero/default values.

16. Defer original compiler bugs.
    - Do not emulate documented incorrect-code cases yet:
      - `ELSEIF a(i)` / `ELSEIF p^`,
      - record pointer parameter not first,
      - `TYPE` fields as call arguments,
      - hexadecimal array dimensions,
      - complex relational expressions in `UNTIL`.
    - Revisit only if a strict bug-compatible mode becomes a goal.
    - Sane default: compile supported cases correctly, and reject unsupported
      complex forms instead of emitting suspect object code.

## Suggested Next Slice

Implement step 15 next:

- optional/omitted argument behavior is implemented.
- next likely slice: revisit complex `UNTIL` lowering or run any missing
  original comparison probes before broadening codegen.

## Progress

- Steps 2 through 4 are implemented:
  - `STRING` is treated as a `CHAR ARRAY` alias in semantic analysis and
    compatible codegen.
  - Global string initializers emit length-prefixed inline storage.
  - Constant indexed reads/writes for global string/`CHAR ARRAY` storage lower
    to direct absolute byte loads/stores.
- Steps 5 and 6 are implemented:
  - Local initialized `STRING` / `CHAR ARRAY` storage emits length-prefixed
    bytes before the owning routine trampoline.
  - `STRING` / `CHAR ARRAY` parameters use pointer-backed indexing through
    `$AE/$AF` for constant and dynamic reads/writes.
- Steps 7 and 8 are implemented:
  - Named global/local arrays and strings pass their base pointer through the
    existing call ABI path.
  - String literal arguments emit inline length-prefixed storage behind a
    jump-over block, then pass the address of the length byte.
  - The broader `SArgs` byte-for-byte prologue gap remains separate.
- Step 9 is implemented:
  - Bare assignment to pointer-backed unsized arrays updates the stored base
    pointer.
  - String literal assignment emits new inline length-prefixed storage behind
    a jump-over block and points the array variable at that storage.
  - Bare assignment to inline sized arrays remains rejected; indexed element
    assignment is still the mutation path for inline storage.
- Steps 10 and 11 are implemented:
  - `TYPE` / `RECORD` layouts with fundamental fields are packed in declaration
    order without padding.
  - Global and local record values reserve inline storage using the packed
    size.
  - Direct record field reads/writes lower to absolute byte loads/stores using
    the field offset.
  - Pointer fields, array fields, and initialized/sized fields are rejected by
    semantic validation.
- Step 12 is implemented:
  - Record pointer field reads/writes load the pointer into `$AE/$AF`.
  - Non-zero field offsets are added to `$AE/$AF`.
  - Field bytes are accessed through `($AE),Y`.
- Step 13 is implemented:
  - Passing a record value to a matching record pointer parameter passes the
    record base address.
  - Passing a record pointer variable still passes the stored pointer value.
  - This works in direct register calls and staged calls.
- Step 14 is implemented:
  - Fixed-address routine declarations such as `PROC Sys=$1234(...)` call the
    absolute target with `JSR`.
  - Caller argument bytes use the same ABI as user routine calls.
  - Empty machine-body declarations such as `PROC Sys=$1234(...) []` are
    accepted and do not emit a local routine body.
- Step 15 is implemented:
  - User and fixed-address calls may omit trailing arguments.
  - Semantic validation rejects only calls with too many arguments.
  - Codegen emits and stages only the supplied argument bytes; omitted bytes are
    left as existing ABI register/zero-page contents.
- Step 16 sane defaults are implemented:
  - Documented original incorrect-code cases are not emulated.
  - `ELSEIF` array/pointer conditions, record pointer parameters after other
    parameters, `TYPE` fields as call arguments, and hexadecimal array
    dimensions are covered as accepted/supported behavior.
  - Unsupported complex `UNTIL` relational expressions are rejected deliberately
    rather than lowered into questionable code.
