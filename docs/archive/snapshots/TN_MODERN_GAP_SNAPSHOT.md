# TN Modern Gap Snapshot

Date: 2026-05-20

Command:

```sh
cargo run -q --bin actionc-compare -- \
  --mode profiles \
  --original samples/tn/original/extracted/TN.COM \
  --max-diffs 0 \
  samples/tn/original/extracted/SRC/TN.ACT.atascii
```

## Current Sizes

- Original Action!: 12127 load bytes, 12115 code bytes, origin `$2C00`
- `compat`: 12060 load bytes, 12048 code bytes, origin `$2C00`
- `modern`: 11242 load bytes, 11230 code bytes, origin `$2C00`

The modern profile is currently 818 bytes smaller than `compat` and 885 bytes
smaller than the captured original TN binary. The latest staged string-literal
argument rule saves 8 bytes in TN by deferring first-argument literal pointer
loads until the final ABI register load.

## Largest Compat-To-Modern Routine Deltas

Positive numbers mean modern is smaller:

```text
delta compat modern routine
  114   1447   1333 SetWin
   98    992    894 Handle
   62    834    772 Copy
   39    285    246 Xloop
   39    128     89 PrintB
   24    285    261 NewDrive
   24    222    198 Fnamecmp
   23    130    107 Tag
   21    265    244 InputLine
   20    216    196 Convert
   18    352    334 Format
   18    234    216 Delete
   17    301    284 PopUp
   16    347    331 Window
   16    108     92 TagAll
   15    176    161 Draw
   15    128    113 InitPannels
   14    328    314 DrawWinFrame
   14    277    263 Range
   14    144    130 MoveMenuBar
```

These deltas are dominated by existing modern wins, not by a new source of
regression. `SetWin`, `Handle`, and `Copy` remain the best routines for finding
generalizable patterns because they contain repeated pointer, call-argument,
and branch shapes.

## Optimization Totals

```text
count bytes kind
  124   372 branch inverted
   39   141 argument store removed
    6    72 pointer reload removed
   33    68 register reload removed
    4    20 argument stack forwarded
   16    16 tail call
    2     6 trampoline elided
    1     0 call fact preserved
```

`call fact preserved` is intentionally a zero-byte observability event. It marks
places where a known-effect call preserved stable zero-page facts that later
rules may use.

## Call-Fact Inspection

Current TN has one `call fact preserved` marker:

```text
call fact preserved  saved 0 bytes  $44F8 SetWin
```

The site is inside `SetWin`, in the assignment:

```action
s(17-i)=Internal(s((17-k)-i))
```

The generated shape saves the destination pointer on the stack, computes the
source pointer, calls `Internal`, restores the destination pointer, and stores
the returned byte from `$A0`. The known-effect call preserves one stable
zero-page fact, but the immediate post-call work is the stack restore plus
`LDA $A0`, so this site does not directly unlock a safe reload/store removal.
The marker is still useful as a breadcrumb for future call-effect work.

## Recommended Next Candidates

1. Continue with prepared-pointer survival across known calls, but only when the
   callee does not write the pointer pair and does not write any dependency
   bytes. TN already has repeated pointer setup wins in `SetWin`, so this is a
   likely next broad rule.
2. Revisit late argument materialization in `SetWin`, `Handle`, and `Copy`,
   especially remaining `$A1 -> X` and `$A2 -> Y` staging where later argument
   setup cannot clobber the target register.
3. Keep the `call fact preserved` marker, but do not force a codegen rule from
   the current `$44F8` site; it is evidence for future call-effect tracking, not
   a direct win.
