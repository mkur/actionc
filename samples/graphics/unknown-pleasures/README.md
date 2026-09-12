# Unknown Pleasures

Start with [unknown-pleasures.act](unknown-pleasures.act) for a small
example that the original Action! 3.6 cartridge compiler accepts. It draws in
full-screen Graphics 8 on an Atari XL/XE: 320 by 192 pixels, white on black.
Press a key after the picture finishes to return to text mode.

With the original Action! cartridge, compile directly from disk so the large
data table stays out of the editor buffer.

[unknown-pleasure-vbxe.act](unknown-pleasure-vbxe.act) is the second renderer,
using VBXE SR320 with quarter-scanline grayscale antialiasing.

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

## Compile with actionc

From the repository root:

```sh
cargo run --bin actionc -- --mode compatibility --runtime standalone --origin '$2C00' \
  samples/graphics/unknown-pleasures/unknown-pleasures.act
```

Use `--runtime cart` when running with the Action! cartridge attached. Both
runtimes are also supported with `--mode optimized` and `--mode mir6502`.

## Data and memory layout

There are two runtime tables: [UPDATA.ACT](UPDATA.ACT) holds 50 evenly spaced
traces with 300 integer heights each, while
[unknown-pleasure-vbxe-data.inc](unknown-pleasure-vbxe-data.inc) holds all 80
traces at quarter-scanline precision. The VBXE renderer selects its 50 traces
at runtime; the beginner table selects them during generation to save 9,000 bytes.

[generate-data.py](generate-data.py) builds both tables directly from the same
pinned CP1919 CSV. It applies the 5/8 vertical scale and rounds each table
independently, so integer heights never undergo a second rounding from VBXE
values. Download the source to the ignored build directory, then regenerate:

```sh
mkdir -p build/unknown-pleasures
curl -fL https://raw.githubusercontent.com/pachadotdev/cp1919/ede09bbbdf6f8d7f88ea8c530866bf6cdb3064b1/cp1919.csv \
  -o build/unknown-pleasures/cp1919.csv
python3 samples/graphics/unknown-pleasures/generate-data.py build/unknown-pleasures/cp1919.csv
```

The generator verifies the CSV's SHA-256 before processing it. Add `--check`
to verify both generated files without writing them. The CSV is only needed
for regeneration; ordinary builds and runtime tests use the two stored tables.

Both tables document the dataset and its provenance. It is a digitization of
Harold Craft's plot, with unverified earlier provenance. The dataset publisher
applies CC0-1.0.

At origin `$2C00`, code/data compiled by the original cartridge compiler occupy
`$2C00-$685D`; the 320-byte `horizon` array occupies
`$685E-$699D`. In Atari800 with the bundled AltirraOS XL and the Action!
cartridge present, the display list starts at `$8036` and the bitmap occupies
`$8150-$9F4F`. This leaves 5,784 bytes between the program's storage and the
display list, without changing RAMTOP or disabling the cartridge.

## Validation

From `tools/vm-runtime-tests`, run:

```sh
cargo test --locked --test unknown_pleasures
```

The test compiles the source with the real Action! 3.6 cartridge ROM, then
compares all 61,440 displayed pixels with actionc output in all three modes and
both runtimes at `$2C00`. It also checks the integer table's recorded checksum,
the selected samples against VBXE's independently rounded heights, screen bounds,
margins, colors, and return to text mode after a keypress.
The VM models graphics calls; it does not test ANTIC scanout. Set
`ACTIONC_UNKNOWN_PLEASURES_ARTIFACT_DIR` to save the cartridge's observed picture.
A native OS render was also checked in Atari800: all 7,680 framebuffer bytes
match the VM image, and the allocated display memory is above code and data.
