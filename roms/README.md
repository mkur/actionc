# ROM Images

These ROM images are checked in because the compiler surveys, VM probes, and
emulator helper rely on them.

| File | Size | SHA-256 | Use |
| --- | ---: | --- | --- |
| `action.rom` | 16400 | `b4a3a399f4f1e8c20f4b1cbc3f6e2fbcef342c36d2c252f903938e93a502c166` | Action! cartridge image used by original-compiler probes and embedded by `actionc-run` |
| `altirraos-xl.rom` | 16384 | `9de5a313fe3946f04fe236a8d3ceacb471fbed4ec5fc5db009732e1169946ccf` | AltirraOS XL/XE 3.11 used by VM/emulator runs and embedded by `actionc-run` |

`altirraos-xl.rom` is the standalone AltirraOS kernel replacement. It was
extracted byte-for-byte from `ROM_altirraos_xl` in Atari800's checked-in
`src/roms/altirraos_xl.c` at commit
`bbe287d6d2c233bc8bad92ed2b2637f6a3859eb6`:

https://github.com/atari800/atari800/blob/bbe287d6d2c233bc8bad92ed2b2637f6a3859eb6/src/roms/altirraos_xl.c

Its copyright and redistribution notice is preserved in
`ALTIRRAOS-LICENSE`. The corresponding source is available in Atari800's
`emuos` directory:

https://github.com/atari800/atari800/tree/bbe287d6d2c233bc8bad92ed2b2637f6a3859eb6/emuos

Most repo scripts discover these files automatically. Override with
`ACTION_VM_CART`, `ACTION_VM_OS`, or `ACTIONC_ATARI800_CART` when comparing
against another ROM.
