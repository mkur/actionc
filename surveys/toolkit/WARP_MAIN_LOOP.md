# WARP.DEM Main Loop Notes

These notes describe the control flow in
`corpora/toolkit/original/extracted/WARP.DEM`. They are source-level notes meant
to make compiler/debugging work easier; they are not a gameplay spec.

## Important idioms

- `BYTE timeR=20` and `BYTE time=20` are absolute-address aliases, not initialized
  locals. Decimal address 20 is the low byte of Atari `RTCLOK`, so loops such as
  `timeR=0 DO UNTIL timeR=1 OD` are one-frame waits.
- `BYTE shipH=53260` in `BlownAway` is also an absolute hardware register alias,
  used as the player collision status for the player's ship.
- `CARD screen=low1` in `Init7` aliases a CARD over adjacent byte locals. The
  routine repeatedly stores the current graphics screen pointer into
  `low1/high1`, then copies those bytes into `YLOCL/YLOCH`.
- `BYTE LOW=LINE, HIGH=LINE+1` aliases the pointer bytes for the indirect
  graphics line pointer used by `Draw7`.
- `PLPTR=PMADR(4)` makes `PLPTR` point at the missile page. Later
  `PLPTR(y)==&mask` and `PLPTR(y)==%mask` clear/set missile bits in player/missile
  memory.

## Startup

`MAIN` performs the hardware and game-state setup:

1. `SerialCtl=3 AudioCtl=0` resets serial/audio control.
2. `DLSetUp()` opens Graphics 7, builds per-scanline lookup tables, installs the
   display-list interrupt, and enables DLI/NMI color effects.
3. `PMGRAPHICS()` allocates player/missile memory below `HiMem`, programs
   `PM_Base`, enables DMA and player/missile graphics, and sets priority.
4. `FOR XX=0 TO 7 DO PMCLEAR(XX) OD` clears four players and four missile masks.
   `PMADR(0..3)` points at player pages; `PMADR(4..7)` maps to the shared missile
   page and `PMCLEAR` masks one missile's bits.
5. The player ship is initialized at `X0=120`, `Y0=120`, color `PCOLR(0)=170`,
   copied into player 0 memory, positioned with `PMHPOS(0)`, and given width 1.
6. `ShowInfo()` draws the text status box, then OS timer 2 is pointed at
   `ScrollColors` and armed with `Timer2=2`. `ScrollColors` keeps rearming itself
   and rotates the `CLRS` color table used by the DLI.

## The repeating loop

The main loop is:

```action
DO
  ShipDraw()
  ShipMove() ShootBack()
  Align()    BALLMOVE()
  FOR Count=1 TO 3
  DO
    timeR=0 DO UNTIL timeR=1 OD
    ShipFly() MissileFire() MissileMove()
    DARKEN() HITBALL() BlownAway()
  OD
OD
```

There are two rates of work:

- Once per outer loop, the game may spawn or move the bitmap mother ship, may
  spawn enemy balls, aligns their velocity toward the player, and animates/moves
  them once.
- Three times per outer loop, after a one-frame wait, it updates direct player
  input, firing, player missile movement, fade/sound effects, missile-vs-enemy
  collision handling, and player-hit explosion handling.

This means player input and missile movement run about three times as often as
the mother-ship AI and enemy ball movement. `ShipDraw` is an exception: when it
spawns the mother ship it contains its own short blocking animation loops, but
those loops still call `ShipFly()` and `MissileMove()` so the player and missiles
remain active during the appearance effect.

## Entity responsibilities

- Player ship:
  - Lives in player/missile player 0 memory.
  - Position is `X0/Y0`, horizontal hardware position is `PMHPOS(0)`.
  - `ShipFly()` reads joystick 0 through `Stick`, `HStick`, and `VStick`, clamps
    the ship to screen bounds, rewrites the player 0 shape at `PMADR(0)+Y0`, and
    updates `PMHPOS(0)`.

- Player missiles:
  - State arrays are `MSTATUS`, `MX/MY`, and `MXOLD/MYOLD`.
  - `MissileFire()` finds an inactive missile, sets its starting position near
    the ship, sets the proper bit in the missile page through `PLPTR`, and writes
    `PMHPOS(i+4)`.
  - `MissileMove()` erases the old missile bit, moves it upward, applies a small
    diagonal spread from the launch point, redraws it in the missile page, updates
    the horizontal register, and calls `TestHit(i)`.

- Mother ship:
  - `ShipStatus=0` means absent. Status 1/2/3 tracks damage states.
  - `ShipDraw()` may spawn it when `Fate>=250`, draws `SHIP`, performs a flashing
    entrance effect, and uses sound channel 1.
  - `ShipMove()` changes `ShipX/ShipY` by `SX/SY`, bounces on bounds or random
    thresholds controlled by `Level` and `Level1`, then draws `SHIP`, `NOLEFT`,
    or `NOENG` with `FastDraw`.
  - `TestHit()` maps missile coordinates into the Graphics 7 coordinate system and
    advances the damage state. Final center hit clears the ship, awards score,
    calls `EraseShip()`, and increases difficulty.

- Enemy balls:
  - State arrays are `BSTAT`, `BX/BY`, and `BXDR/BYDR`.
  - `ShootBack()` occasionally spawns a ball from the mother ship into player
    pages 1..3, chooses a color, writes `BALL1`, and positions the player.
  - `Align()` updates each active ball's velocity to drift toward `X0/Y0`.
  - `BALLMOVE()` toggles between `BALL1` and `BALL2`, applies velocity, clears
    off-screen balls, and updates `PMHPOS(i)`.
  - `HITBALL()` checks missile collision registers at `$D008`, clears the hit
    missile and ball, starts a `Color4` explosion fade, clears hit latches, and
    awards score.

- Player death:
  - `BlownAway()` tests `shipH` and returns if there is no collision.
  - On collision it disables player width, clears missiles, turns all four
    player/missile objects into explosion balls moving diagonally away, waits
    until they leave the play area, decrements `NumShips`, and either calls
    `EndGame()` or respawns the player ship.

## Compiler-sensitive areas

- The program relies heavily on absolute-address aliases that look like
  assignments. Treating `BYTE timeR=20` as storage initialized to 20 would turn
  frame waits into infinite loops.
- Several routines use pointer-cell byte arrays (`PLPTR`, `PLAYADR`, `LINE`) and
  expect both byte and CARD views over the same storage to work.
- `FastDraw` uses a CARD `lctr3` for picture offsets:
  `lctr3=(lctr1+1)*width-1`. Even when current sprite widths stay under 256,
  codegen must not generally force the high byte of the runtime multiply result
  to zero before the subtract.
- `PMADR` maps `N>=4` to the missile page and otherwise maps 0..3 to player
  pages 0..3 by incrementing `N` before multiplying by `$100`. This routine is a
  good place to inspect CARD-vs-pointer behavior.
