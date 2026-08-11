# Action! Cartridge Notice

`action.rom` is the Action! 3.6 OSS 16K cartridge image. Action! was written by
Clinton W. Parker and originally published in 1983 by Action Computer Services
and Optimized Systems Software.

The corresponding assembly source is licensed under the GNU General Public
License, version 3 or any later version (`GPL-3.0-or-later`). The complete GPLv3
license text is provided in this repository's top-level `LICENSE` file.

## Corresponding Source

The maintained upstream source repository is:

```text
https://git.code.sf.net/p/atari-action/code
```

This cartridge was verified against revision:

```text
0b8bcedb9951acd84a1814d60e2eaeb0a93dd45f
```

A browsable copy of that revision is available at:

```text
https://sourceforge.net/p/atari-action/code/ci/0b8bcedb9951acd84a1814d60e2eaeb0a93dd45f/tree/
```

The cartridge source and GPL notice are under `JAC/src/`, including:

```text
JAC/src/ACTION-ROM-OSS-16k.asm
JAC/src/GPL.txt
```

The upstream project summary currently labels the project GPLv2, but the
corresponding source files and the GPL text distributed with them explicitly
license Action! under GPL version 3 or any later version. This notice follows
the license terms present in the corresponding source distribution.

## Rebuilding

At the pinned revision, `JAC/build/Make-ACTION.bat` invokes MADS on
`JAC/src/ACTION-ROM-OSS-16k.asm`. The build compares the generated 16K ROM
byte-for-byte with `JAC/ref/rom/ACTION-36-ROM-OSS.rom`, then creates an Atari CAR
file using cartridge type 15.

The reference ROM has this identity:

```text
size:    16384 bytes
SHA-256: 37b6366236eccd1dd52b12f38ad022f192689798069d3c8ca66be0dc9ac1397f
```

## Bundled Image Identity

The bundled file has this identity:

```text
size:    16400 bytes
SHA-256: b4a3a399f4f1e8c20f4b1cbc3f6e2fbcef342c36d2c252f903938e93a502c166
```

Its first 16 bytes are the standard Atari CAR header for cartridge type 15.
After removing that header, the remaining 16384 bytes match the pinned
upstream reference ROM byte-for-byte.

Public binary releases should preserve this notice and provide durable access
to a source snapshot of the pinned revision alongside the binary assets.
