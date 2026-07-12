# Runtime And Builtin Effect Map

This is the source-facing map for routines whose side effects matter to
`actionc` code generation. The encoded source of truth is split across
`src/codegen/runtime.rs` for internal runtime helpers and `src/codegen/model.rs`
for Action! routines and OS entry points.

`RoutineEffects` currently tracks:

- whether effects are known at all
- whether `A`, `X`, or `Y` are explicitly preserved
- zero-page writes
- bounded absolute writes
- unknown absolute writes

The model does not track processor flags, stack depth changes, read effects, or
semantic effects such as I/O device state. If a routine has known effects but no
registers are marked preserved, the compiler treats `A/X/Y` as clobbered.

## Runtime Zero Page

These names are the compiler-owned scratch slots that appear in effect maps:

| Name | Bytes | Use |
| --- | --- | --- |
| `AFLAST` | `$82-$83` | Action floating/scratch helper state |
| `AFCUR` | `$84-$85` | helper operand / Action floating scratch |
| `AFSIZE` | `$86-$87` | Action floating/scratch helper state |
| `ARGS` | `$A0-$AF` | argument/result tracking window |
| `VALUE_TEMP` | `$AA` | compiler temporary inside the `ARGS` window |
| `ELEMENT_ADDR` | `$AC-$AD` | compiler pointer temporary inside the `ARGS` window |
| `ARRAY_ADDR` | `$AE-$AF` | compiler pointer temporary inside the `ARGS` window |
| `DEVICE` | `$B7` | Action device byte |
| `ADDR` | `$C0-$C1` | compiler pointer/value temporary |
| `TOKEN` | `$C2` | Action token / helper scratch |

The named compiler temporaries intentionally overlap the high end of the
`ARGS` tracking window. Effect code therefore records both the specific
zero-page byte writes and derived flags such as "writes args" or "writes
array address."

## Runtime Helper Targets

`samples/tn/modern/LIB.ACT` carries standalone implementations of the Action!
arithmetic/runtime helper routines. These byte blocks match the cartridge helper
contracts used through the vector slots at `$04E4-$04EF`, so `actionc` uses them
as the behavioral reference for both cartridge and standalone helper calls.

| Helper | Standalone slot | Cartridge address | Purpose |
| --- | --- | --- | --- |
| `r_Lsh` | `$04E4` | `$B5C0` | left shift |
| `r_Rsh` | `$04E6` | `$A0E6` | right shift |
| `r_Mul` | `$04E8` | `$A000` | multiply |
| `r_Div` | `$04EA` | `$A090` | divide |
| `r_Mod` | `$04EC` | `$A0DE` | remainder |
| `r_Par` / `SArgs` | `$04EE` | `$A0F5` | stack argument frame copy |

## Runtime Helper Calling Shape

The compiler passes the left operand in `A/X`. For multiply, divide, and
remainder, the right operand is placed in `$84/$85`. For shifts, the shift count
is placed in `$84`. Arithmetic helper results return in `A/X`.

`r_Par` is special: it is called from a routine prologue when three or more
argument bytes must be copied from the caller argument area into the callee's
local frame. The inline bytes after the `JSR` describe the destination frame and
byte count.

## Trusted Runtime Helper Effects

The state tracker treats these calls as known-effect barriers:

| Helper | Preserved registers | Zero-page writes | Absolute writes |
| --- | --- | --- | --- |
| `r_Lsh` / `r_Rsh` | none | `$85` | none |
| `r_Mul` | none | `$82-$87`, `$C0-$C2` | none |
| `r_Div` / `r_Mod` | none | `$82-$87`, `$C2` | none |
| `r_Par` / `SArgs` | none | `$82-$85`, `$A0-$A2` | unknown destination frame |

These ranges are covered by
`runtime_helper_effects_match_action_scratch_ranges`.

Important clobber note: `r_Mul` writes `$C0-$C2`, while `r_Div` and `r_Mod`
write `$C2` but not `$C0-$C1`. Codegen must not preserve an address or operand
in `ADDR` across `*`; it may preserve `ADDR` across `/`, `DIV`, or `MOD` with
respect to helper effects, but only if no surrounding expression call or
materialization writes it.

## Encoded OS Effects

Only two OS entry points are currently modeled with trusted effects:

| Address | Name / use | Preserved registers | Zero-page writes | Absolute writes |
| --- | --- | --- | --- | --- |
| `$E456` | Atari OS `CIOV` | none | none | `$0340-$03BF` |
| `$F2F8` | TN `Getchar` OS helper | none | none | none |

All other numeric system calls are unknown unless a source annotation supplies
effects.

## Cartridge Builtin Coverage

The compiler knows the signatures and cartridge addresses for the Action!
builtins below, but their effects are still modeled as unknown. Unknown effects
are safe for state tracking because they invalidate cached facts broadly, but
they are not yet a usable clobber map for code quality or focused preservation.

| Builtin group | Names |
| --- | --- |
| Graphics | `Graphics`, `SetColor`, `Plot`, `DrawTo`, `Position`, `Locate`, `Point`, `Fill` |
| Console/device I/O | `Print`, `PrintE`, `PrintF`, `PrintB`, `PrintBE`, `PrintBD`, `PrintBDE`, `PrintC`, `PrintCE`, `PrintCD`, `PrintCDE`, `PrintD`, `PrintDE`, `PrintI`, `PrintIE`, `PrintID`, `PrintIDE`, `InputS`, `InputSD`, `InputMD`, `InputB`, `InputBD`, `InputC`, `InputCD`, `InputI`, `InputID`, `Put`, `PutE`, `PutD`, `PutDE`, `GetD`, `Open`, `Close`, `XIO` |
| Strings | `SCompare`, `SCopy`, `SCopyS`, `SAssign`, `ValB`, `ValC`, `ValI` |
| Memory | `Zero`, `SetBlock`, `MoveBlock`, `Peek`, `PeekC`, `Poke`, `PokeC` |
| Input/sound/misc | `Break`, `Rand`, `Note`, `Sound`, `SndRst`, `Paddle`, `PTrig`, `Stick`, `STrig`, `Error` |

## Annotation Path

Source annotations can add facts and effects for user routines. The effect path
is described in `docs/ACTIONC_ANNOTATIONS.md`; once a routine has trusted
effects, the code generator can preserve cached memory facts and prepared
pointers across the call when the written ranges do not overlap.

## Probe Backlog

The missing piece is an executable survey for the cartridge builtins. A useful
probe should capture, for each builtin:

- `A/X/Y` before and after
- processor flags if they become relevant to codegen
- zero-page bytes that differ after the call
- bounded absolute ranges touched by known OS control blocks or caller buffers
- whether writes depend on arguments such as source/destination pointers

After probing, the results should be added to `system_effects_for_address` or to
the builtin table in `src/codegen/model.rs`, with tests that assert the specific
zero-page and absolute-write ranges.
