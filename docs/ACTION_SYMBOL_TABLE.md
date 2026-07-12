# Action! Symbol And Library Lookup Evidence

This note records the current evidence for Action!'s user symbol tables and the
separate cartridge system-library lookup path. For now, the goal is modest:
record only directly evidenced system-library entry points.

## Sources

- `corpora/action-runtime/extracted/ST.ACT`
- `corpora/action-runtime/extracted/ST.DOC`
- `corpora/action-runtime/extracted/BIGST.ACT`
- `corpora/action-runtime/extracted/SYS.ACT`
- VM probes, including `graphics_calls.act`
- Action! manual section 1.1.2, "Symbol Table Searches"
- `docs/resident_library.md`, resident library routine catalog extracted from
  the manual

## Table Roots

`ST.ACT` is the strongest source because it is an official Action! symbol table
lister. It calls:

- `DumpST($B1)` for global declarations
- `DumpST($B3)` for local declarations

`DumpST` treats the argument as a `CARD POINTER`. In other words:

- `$B1/$B2` holds the address of the global symbol table index.
- `$B3/$B4` holds the address of the local symbol table index.

The index has two parallel 256-byte pages:

- `stHigh = base^`
- `stLow = stHigh + 256`

For each slot `i`, if `stHigh(i)` is nonzero, the symbol-name address is:

```text
address = stLow(i) + 256 * stHigh(i)
```

The official lister sorts entries lexically before printing, but the index
itself is just the lookup/index structure.

## Entry Layout

`ST.ACT` defines:

```action
TYPE ENTRY =
[
  ; STRING name(?)
  BYTE vtype
  CARD adr
  BYTE numargs
  ; BYTE ARRAY argTypes(8)
]
```

The name pointer points to an Action string:

```text
name_len:  byte
name:      name_len bytes
entry:     immediately follows name
```

Therefore:

```text
entry = name_address + memory[name_address] + 1
vtype = entry[0]
adr = entry[1] + 256 * entry[2]
numargs = entry[3]
argTypes = entry + 3
```

The `argTypes` indexing in `ST.ACT` starts at 1, so the first printed argument
type is at `entry + 4`.

## Type Encoding

From `ST.ACT`:

- `vtype = $88` means undeclared; skip it.
- `vtype = 27` means `DEFINE`; the define string starts at `entry + 3`.
- `vtype = 39` means user `TYPE`.
- `vtype & $07` gives the fundamental base type:
  - `1` = `CHAR`
  - `2` = `BYTE`
  - `3` = `INT`
  - `4` = `CARD`
- `vtype & $10` marks `ARRAY`.
- `vtype & $40` marks `PROC` or `FUNC`.
- `(vtype & $F7) == $C0` marks `PROC`; otherwise it is a `FUNC`.
- For `vtype < 128`, `ST.ACT` treats the entry as record-related.
  - `(vtype & 7) == 0` means `RECORD`.
  - `vtype & 8` marks `RECORD POINTER`.
  - nonzero low type bits mark record fields.

For routine arguments, `ST.ACT` prints `argTypes(i) % $80`. In Action!, `%` is
bitwise OR, so this sets the high bit before applying the same type decoder.
That makes scalar argument types decode as ordinary scalar types instead of
record-field entries.

## Library Lookup Path

The manual says the compiler searches names in this order:

1. local symbol table;
2. global symbol table;
3. built-in cartridge system library.

Only after all three fail does Action! report an undefined symbol. Therefore
official library routines should not be expected to appear as ordinary valid
entries in the local/global RAM hash tables.

VM snapshots match this:

- A plain boot snapshot decoded no `$B1` global entries.
- A snapshot after compiling `graphics_calls.act` decoded only the user global:

```text
Main  $3000  PROC
```

The name heap around `$9400` did contain `color` and `Plot`, but both had
`vtype=$88`, which `ST.ACT` explicitly treats as undeclared and skips. The bytes
following those names did not match the emitted routine target for `Plot`
(`$A6C3`).

So `$B1/$B2` and `$B3/$B4` are useful for user symbols, but the official system
library is resolved through a separate cartridge lookup path.

## Manual Library Catalog Cross-Check

`docs/resident_library.md` lists the resident system library surface. It
confirms the broad set of cartridge routines, but it does not give entry-point
addresses. Addresses still need VM probes, cartridge table extraction, or
runtime-library evidence.

Current `actionc` state against that catalog:

- Semantics seeds most listed resident names as built-ins.
- Semantics currently does not seed `StrB`, `StrC`, `StrI`, or `Error`.
- Codegen has cartridge entry/ABI information only for the modeled subset in
  `src/codegen.rs`. Other seeded library names are recognized semantically but
  still need codegen ABI modeling before direct cartridge calls can compile.
- Manual placeholders such as `<string>`, `<filestring>`, and `<data>` are not
  full type signatures. They should be treated as compatibility hints and
  verified with probes before tightening semantic checks.
- The catalog supports the current design choice that a built-in needs both a
  semantic signature and a codegen target. The original cartridge likely carries
  enough metadata to do both, but `actionc` still models that metadata manually.

The broad resident entry-point probes are:

- `resident_output.act` -> `RESOUT.COM`
- `resident_input.act` -> `RESIN.COM`
- `resident_file.act` -> `RESFILE.COM`
- `resident_graphics_game.act` -> `RESGFX.COM`
- `resident_string_convert.act` -> `RESSTR.COM`
- `resident_misc_memory.act` -> `RESMISC.COM`

## Confirmed Library Entry Points

These addresses are evidence-backed by VM probes against the original cartridge.
They should be treated as cartridge-version facts until broader extraction or
more probes say otherwise.

| Name | Kind | Signature / Location | Entry / Address | Evidence |
| --- | --- | --- | --- | --- |
| `Print` | `PROC` | `PROC Print(<string>)` | `$A47F` | `resident_output.act` emits `JSR $A47F` |
| `PrintE` | `PROC` | `PROC PrintE(<string>)` | `$A46C` | `resident_output.act` emits `JSR $A46C` |
| `PrintD` | `PROC` | `PROC PrintD(BYTE d,<string>)` | `$A486` | `resident_output.act` emits `JSR $A486`; `SYS.ACT` declares the signature |
| `PrintDE` | `PROC` | `PROC PrintDE(BYTE d,<string>)` | `$A473` | `resident_output.act` emits `JSR $A473` |
| `PrintB` | `PROC` | `PROC PrintB(BYTE n)` | `$A4E4` | `resident_output.act` emits `JSR $A4E4` |
| `PrintBE` | `PROC` | `PROC PrintBE(BYTE n)` | `$A4EC` | `resident_output.act` emits `JSR $A4EC` |
| `PrintBD` | `PROC` | `PROC PrintBD(BYTE d,n)` | `$A4F4` | `resident_output.act` emits `JSR $A4F4` |
| `PrintBDE` | `PROC` | `PROC PrintBDE(BYTE d,n)` | `$A508` | `resident_output.act` emits `JSR $A508` |
| `PrintC` | `PROC` | `PROC PrintC(CARD n)` | `$A4E6` | `resident_output.act` emits `JSR $A4E6` |
| `PrintCE` | `PROC` | `PROC PrintCE(CARD n)` | `$A4EE` | `resident_output.act` emits `JSR $A4EE` |
| `PrintCD` | `PROC` | `PROC PrintCD(BYTE d,CARD n)` | `$A4F6` | `resident_output.act` emits `JSR $A4F6` |
| `PrintCDE` | `PROC` | `PROC PrintCDE(BYTE d,CARD n)` | `$A50A` | `resident_output.act` emits `JSR $A50A` |
| `PrintI` | `PROC` | `PROC PrintI(INT n)` | `$A512` | `resident_output.act` emits `JSR $A512` |
| `PrintIE` | `PROC` | `PROC PrintIE(INT n)` | `$A536` | `resident_output.act` emits `JSR $A536` |
| `PrintID` | `PROC` | `PROC PrintID(BYTE d,INT n)` | `$A519` | `resident_output.act` emits `JSR $A519` |
| `PrintIDE` | `PROC` | `PROC PrintIDE(BYTE d,INT n)` | `$A53C` | `resident_output.act` emits `JSR $A53C` |
| `PrintF` | `PROC` | `PROC PrintF(STRING f,CARD a1,a2,a3,a4,a5)` | `$A3CC` | `io_builtin_calls.act` emits `JSR $A3CC`; `SYS.ACT` declares the signature |
| `Put` | `PROC` | `PROC Put(CHAR c)` | `$A4CE` | `resident_output.act` emits `JSR $A4CE` |
| `PutE` | `PROC` | `PROC PutE()` | `$A4CC` | `resident_output.act` emits `JSR $A4CC` |
| `PutD` | `PROC` | `PROC PutD(BYTE d,CHAR c)` | `$A4D1` | `resident_output.act` emits `JSR $A4D1` |
| `PutDE` | `PROC` | `PROC PutDE(BYTE dev)` | `$A4DA` | `resident_output.act` emits `JSR $A4DA` |
| `InputB` | `BYTE FUNC` | `BYTE FUNC InputB()` | `$A588` | `resident_input.act` emits `JSR $A588` |
| `InputBD` | `BYTE FUNC` | `BYTE FUNC InputBD(BYTE d)` | `$A58A` | `resident_input.act` emits `JSR $A58A` |
| `InputC` | `CARD FUNC` | `CARD FUNC InputC()` | `$A588` | `resident_input.act` emits `JSR $A588` |
| `InputCD` | `CARD FUNC` | `CARD FUNC InputCD(BYTE d)` | `$A58A` | `resident_input.act` emits `JSR $A58A` |
| `InputI` | `INT FUNC` | `INT FUNC InputI()` | `$A588` | `resident_input.act` emits `JSR $A588` |
| `InputID` | `INT FUNC` | `INT FUNC InputID(BYTE d)` | `$A58A` | `resident_input.act` emits `JSR $A58A` |
| `InputS` | `PROC` | `PROC InputS(<string>)` | `$A48C` | `resident_input.act` emits `JSR $A48C` |
| `InputSD` | `PROC` | `PROC InputSD(BYTE d,<string>)` | `$A493` | `resident_input.act` emits `JSR $A493` |
| `InputMD` | `PROC` | `PROC InputMD(BYTE d,<string>,BYTE m)` | `$A499` | `resident_input.act` emits `JSR $A499`; `SYS.ACT` declares the signature |
| `GetD` | `CHAR FUNC` | `CHAR FUNC GetD(BYTE d)` | `$A4AD` | `resident_input.act` emits `JSR $A4AD` |
| `Open` | `PROC` | `PROC Open(BYTE d,<filestring>,BYTE m,a2)` | `$A444` | `resident_file.act` emits `JSR $A444` |
| `Close` | `PROC` | `PROC Close(BYTE d)` | `$A479` | `resident_file.act` emits `JSR $A479` |
| `XIO` | `PROC` | `PROC XIO(BYTE d,x,c,a1,a2,<filestring>)` | `$A4DE` | `resident_file.act` emits `JSR $A4DE` |
| `Note` | `PROC` | `PROC Note(BYTE d,CARD POINTER s,BYTE POINTER o)` | `$A60D` | `resident_file.act` emits `JSR $A60D` |
| `Point` | `PROC` | `PROC Point(BYTE d,CARD s,BYTE o)` | `$A634` | `resident_file.act` emits `JSR $A634` |
| `Graphics` | `PROC` | `PROC Graphics(BYTE mode)` | `$A654` | `resident_graphics_game.act` emits `JSR $A654` |
| `SetColor` | `PROC` | `PROC SetColor(BYTE r,h,l)` | `$A6CE` | `resident_graphics_game.act` emits `JSR $A6CE` |
| `Plot` | `PROC` | `PROC Plot(CARD c,BYTE r)` | `$A6C3` | `resident_graphics_game.act` emits `JSR $A6C3`; `SYS.ACT` declares the signature |
| `DrawTo` | `PROC` | `PROC DrawTo(CARD c,BYTE r)` | `$A68C` | `resident_graphics_game.act` emits `JSR $A68C`; `SYS.ACT` declares the signature |
| `Fill` | `PROC` | `PROC Fill(CARD c,BYTE r)` | `$A6E9` | `resident_graphics_game.act` emits `JSR $A6E9` |
| `Position` | `PROC` | `PROC Position(CARD c,BYTE r)` | `$A6AE` | `resident_graphics_game.act` emits `JSR $A6AE` |
| `Locate` | `BYTE FUNC` | `BYTE FUNC Locate(CARD c,BYTE r)` | `$A6BB` | `resident_graphics_game.act` emits `JSR $A6BB` |
| `Sound` | `PROC` | `PROC Sound(BYTE v,p,d,vol)` | `$A704` | `resident_graphics_game.act` emits `JSR $A704` |
| `SndRst` | `PROC` | `PROC SndRst()` | `$A721` | `resident_graphics_game.act` emits `JSR $A721` |
| `Paddle` | `BYTE FUNC` | `BYTE FUNC Paddle(BYTE p)` | `$AD37` | `resident_graphics_game.act` emits `JSR $AD37` |
| `PTrig` | `BYTE FUNC` | `BYTE FUNC PTrig(BYTE p)` | `$A737` | `resident_graphics_game.act` emits `JSR $A737` |
| `Stick` | `BYTE FUNC` | `BYTE FUNC Stick(BYTE p)` | `$A74E` | `resident_graphics_game.act` emits `JSR $A74E` |
| `STrig` | `BYTE FUNC` | `BYTE FUNC STrig(BYTE p)` | `$AD2F` | `resident_graphics_game.act` emits `JSR $AD2F` |
| `SCompare` | `INT FUNC` | `INT FUNC SCompare(<string>,<string>)` | `$A864` | `resident_string_convert.act` emits `JSR $A864`; `SYS.ACT` declares the signature |
| `SCopy` | `PROC` | `PROC SCopy(<dest>,<source>)` | `$A898` | `resident_string_convert.act` emits `JSR $A898` |
| `SCopyS` | `PROC` | `PROC SCopyS(<dest>,<source>,BYTE start,stop)` | `$A8AF` | `resident_string_convert.act` emits `JSR $A8AF` |
| `SAssign` | `PROC` | `PROC SAssign(<dest>,<source>,BYTE start,stop)` | `$A8D8` | `resident_string_convert.act` emits `JSR $A8D8` |
| `StrB` | `PROC` | `PROC StrB(BYTE n,<string>)` | `$A544` | `resident_string_convert.act` emits `JSR $A544` |
| `StrC` | `PROC` | `PROC StrC(CARD n,<string>)` | `$A54C` | `resident_string_convert.act` emits `JSR $A54C` |
| `StrI` | `PROC` | `PROC StrI(INT n,<string>)` | `$A55B` | `resident_string_convert.act` emits `JSR $A55B` |
| `ValB` | `BYTE FUNC` | `BYTE FUNC ValB(<string>)` | `$A59A` | `resident_string_convert.act` emits `JSR $A59A` |
| `ValC` | `CARD FUNC` | `CARD FUNC ValC(<string>)` | `$A59A` | `resident_string_convert.act` emits `JSR $A59A` |
| `ValI` | `INT FUNC` | `INT FUNC ValI(<string>)` | `$A59A` | `resident_string_convert.act` emits `JSR $A59A` |
| `Rand` | `BYTE FUNC` | `BYTE FUNC Rand(BYTE r)` | `$A6F1` | `resident_misc_memory.act` emits `JSR $A6F1` |
| `Break` | `PROC` | `PROC Break()` | `$A7DA` | `resident_misc_memory.act` emits `JSR $A7DA` |
| `Error` | `PROC` | `PROC Error(BYTE e)` | `$04CB` | `resident_misc_memory.act` emits `JSR $04CB` |
| `Peek` | `BYTE FUNC` | `BYTE FUNC Peek(CARD a)` | `$A767` | `resident_misc_memory.act` emits `JSR $A767` |
| `PeekC` | `CARD FUNC` | `CARD FUNC PeekC(CARD a)` | `$A767` | `resident_misc_memory.act` emits `JSR $A767` |
| `Poke` | `PROC` | `PROC Poke(CARD a,BYTE v)` | `$A777` | `resident_misc_memory.act` emits `JSR $A777` |
| `PokeC` | `PROC` | `PROC PokeC(CARD a,CARD v)` | `$A781` | `resident_misc_memory.act` emits `JSR $A781` |
| `Zero` | `PROC` | `PROC Zero(BYTE POINTER a,CARD s)` | `$A78A` | `resident_misc_memory.act` emits `JSR $A78A`; `SYS.ACT` declares the signature |
| `SetBlock` | `PROC` | `PROC SetBlock(BYTE POINTER a,CARD s,BYTE v)` | `$A790` | `resident_misc_memory.act` emits `JSR $A790` |
| `MoveBlock` | `PROC` | `PROC MoveBlock(BYTE POINTER d,s,CARD sz)` | `$A7B3` | `resident_misc_memory.act` emits `JSR $A7B3` |
| `color` | byte storage | graphics color latch | `$02FD` | `graphics_calls.act` stores `color=3` as `STA $02FD`; `SYS.ACT` `Plot` implementation also reads `$2FD` |
| `device` | byte storage | default I/O device/channel | `$B7` | `put_device_call.act` VM probe compiles `d=device` as `LDA $B7` |

## Probe Details

`SYS.ACT` declares:

```action
PROC Plot=*(CARD c,BYTE r)
  [$20Pos1 $6A9 $AE$2FD $4CPutD]
```

The `graphics_calls.act` probe compiled by the cartridge emitted:

```text
JSR $A6C3
```

for `Plot(1,2)`, and stored `color` at `$02FD`. This should be treated as a
confirmation probe.

## Open Questions

- Where the cartridge stores or computes official library routine metadata.
- Whether the official library table is a ROM table, a banked table, or custom
  lookup code rather than an Action! symbol table.
- Whether different Action! cartridge versions have different resident addresses.
- Whether `color` is represented in the cartridge library table or is handled as
  a conventional runtime/OS location.
