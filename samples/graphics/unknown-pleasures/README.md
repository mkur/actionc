# Unknown Pleasures

Start with [unknown-pleasures-cart.act](unknown-pleasures-cart.act) for a small
example that the original Action! 3.6 cartridge compiler accepts. It draws in
full-screen Graphics 8 on an Atari XL/XE: 320 by 192 pixels, white on black.
Press a key after the picture finishes to return to text mode.

## Run the prepared disk in Altirra

The repository includes a ready-to-run [disk image](bin/pleasures.atr) with the
source, data, and two executables:

| File | Compiler | Runtime requirement |
| --- | --- | --- |
| `STANDALN.COM` | actionc, compatibility mode | Runs without an Action! cartridge |
| `PLEASURE.COM` | Original Action! 3.6 cartridge | Requires the Action! cartridge to stay attached |

For a normal MyDOS launch without Action!, choose `L` and enter `STANDALN.COM`.
The same [standalone executable](bin/unknown-pleasures-standalone.xex) is also
included for direct loading. Both builds use the same
[beginner source](unknown-pleasures-cart.act) and [data table](UPDATA.ACT).

To run `PLEASURE.COM`, attach `roms/action.rom` as the cartridge before booting.
The disk does not supply the cartridge runtime: `Graphics`, `Plot`, and other
helpers call into its ROM. Loading that file without Action!, or with a different
cartridge, can jump into unrelated code or empty memory and crash.

## Source overview

The program uses ordinary `BYTE`, `CARD`, arrays, loops, and `Plot`. Its three
short procedures separate the drawing from the screen setup. There are no
modules, pointers, machine code, or direct bitmap writes to learn first.

## How the beginner version works

1. `Main` opens the graphics screen, chooses white ink, and prepares `horizon`.
2. `DrawPulses` reads all 300 heights for each of 50 traces. The first baseline is
   near the bottom, at row 181. Each later baseline is three rows higher.
3. `DrawColumn` joins neighboring heights with a short vertical line. It draws
   only above the highest point already drawn in that column. That makes the
   nearer traces hide parts of the traces behind them.

Screen row numbers increase downwards, so a taller pulse uses a smaller row
number. `horizon(x)` remembers the topmost occupied row in column `x`; its
initial value, 192, is just below the screen. `limit` is that value for the
column currently being drawn.

`CH` is the Atari OS's last-key value; `$FF` means no key is waiting.
Setting `ATRACT` to zero prevents the OS from changing the display colors
while the picture is on screen.

[UPDATA.ACT](UPDATA.ACT) holds the measurements. Each value has a bias of four
added to it so the table can use unsigned bytes. The drawing undoes this with
`y=baseline+4-pulseData(index)`. `index` and `x` are `CARD` values because the
table contains 15,000 bytes and the screen is 320 pixels wide. Heights and the
trace counter still fit in a `BYTE`.

For a first experiment, change the final `14` in `SetColor(1,0,14)` to lower the
line brightness. Keep the drawing dimensions and spacing as written until you
understand the bounds: the data generator checks that every computed row stays
within 0..191. The supplied data uses rows 5 through 182.

## Compile with the original cartridge

Prepare both files with Atari line endings and short disk filenames. The
cartridge compiler needs two `SET` directives before the source to place code
and data at `$2C00`. This command adds them to the disk copy:

```sh
python3 - <<'PY'
from pathlib import Path
project = Path('samples/graphics/unknown-pleasures')
output = Path('build/unknown-pleasures-cart')
output.mkdir(parents=True, exist_ok=True)
source = (project / 'unknown-pleasures-cart.act').read_bytes()
source = b'SET $E=$2C00\nSET $491=$2C00\n' + source
(output / 'PLEASURE.ACT').write_bytes(source.replace(b'\n', b'\x9b'))
data = (project / 'UPDATA.ACT').read_bytes()
(output / 'UPDATA.ACT').write_bytes(data.replace(b'\n', b'\x9b'))
PY
```

Put `PLEASURE.ACT` and `UPDATA.ACT` on the same Atari disk in drive 1. At the
Action! monitor, compile directly from disk, then run:

```text
C "D:PLEASURE.ACT"
R
```

Compiling from disk keeps the large measurement table out of the editor buffer.
After returning to the monitor, `W "D:PLEASURE.COM"` saves the compiled program.
The saved program needs the Action! cartridge runtime.

## Compile with actionc

From the repository root:

```sh
cargo run --bin actionc -- --mode compatibility --runtime cart --origin '$2C00' \
  samples/graphics/unknown-pleasures/unknown-pleasures-cart.act
```

This creates `unknown-pleasures-cart.xex`, which runs with the Action! cartridge.
Use `--runtime standalone` to build a version that runs without it. Both
runtimes are also supported with `--mode optimized` and `--mode mir6502`.
Use the repository source for actionc, with the origin supplied on its command
line. The disk copy's cartridge `SET` directives are kept separate because they
can conflict with actionc's placement of standalone runtime helpers.

Rebuild the standalone executable stored in this repository with:

```sh
cargo run --bin actionc -- --mode compatibility --runtime standalone --origin '$2C00' \
  --output samples/graphics/unknown-pleasures/bin/unknown-pleasures-standalone.xex \
  samples/graphics/unknown-pleasures/unknown-pleasures-cart.act
```

To refresh the standalone executable on the stored disk while preserving its
other files:

```sh
mkdir -p build/unknown-pleasures-cart
cargo run --manifest-path crates/atrcopy-rs/Cargo.toml --bin atrcopy-rs -- \
  samples/graphics/unknown-pleasures/bin/pleasures.atr add \
  -o build/unknown-pleasures-cart/pleasures-updated.atr \
  samples/graphics/unknown-pleasures/bin/unknown-pleasures-standalone.xex=STANDALN.COM
cp build/unknown-pleasures-cart/pleasures-updated.atr \
  samples/graphics/unknown-pleasures/bin/pleasures.atr
```

## Data and memory layout

The beginner table keeps the same 50 traces displayed by
[unknown-pleasures.act](unknown-pleasures.act), with all 300 horizontal samples
and unchanged heights. Selecting these traces ahead of time leaves out the 30
unused traces and reduces the stored table from 24,000 to 15,000 bytes. The
drawing itself retains the full horizontal resolution. Regenerate the table with:

```sh
python3 samples/graphics/unknown-pleasures/generate-cart-data.py
```

The source table documents the pinned CP1919 dataset and its provenance. It
is a digitization of Harold Craft's plot, with unverified earlier provenance.
The dataset publisher applies CC0-1.0. The beginner variant uses this same data.

At origin `$2C00`, the original cartridge compiler produces a 15,466-byte load
file. Its code/data occupy `$2C00-$685D`; the 320-byte `horizon` array occupies
`$685E-$699D`. In Atari800 with the bundled AltirraOS XL and the Action!
cartridge present, the display list starts at `$8036` and the bitmap occupies
`$8150-$9F4F`. This leaves 5,784 bytes between the program's storage and the
display list, without changing RAMTOP or disabling the cartridge.

The advanced `unknown-pleasures.act` uses direct bitmap access and selects the
50 traces at runtime from its complete 80-trace table.
[unknown-pleasure-vbxe.act](unknown-pleasure-vbxe.act) uses VBXE SR320 and
quarter-scanline grayscale antialiasing.

## Validation

From `tools/vm-runtime-tests`, run:

```sh
cargo test --locked --test unknown_pleasures_cart
```

The test compiles the source with the real Action! 3.6 cartridge ROM, then
compares all 61,440 displayed pixels with actionc output in all three modes and
both runtimes at `$2C00`. It also checks the selected data against the source table,
screen bounds, margins, colors, and return to text mode after a keypress.
The VM models graphics calls; it does not test ANTIC scanout. Set
`ACTIONC_UNKNOWN_PLEASURES_ARTIFACT_DIR` to save the cartridge's observed picture.
A native OS render was also checked in Atari800: all 7,680 framebuffer bytes
match the VM image, and the allocated display memory is above code and data.
