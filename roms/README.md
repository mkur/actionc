# ROM Images

These ROM images are checked in because the compiler surveys and VM probes rely
on this exact Action! cartridge build.

| File | Size | SHA-256 | Use |
| --- | ---: | --- | --- |
| `action.rom` | 16400 | `b4a3a399f4f1e8c20f4b1cbc3f6e2fbcef342c36d2c252f903938e93a502c166` | Action! cartridge image used by original-compiler probes |
| `rev02.rom` | 16384 | `a77050b2d81db2d11eaa3dbafd8ec2531b478abcc3bcc4b0d846b634e885edb1` | Atari OS ROM used by VM runs |

Most repo scripts discover these files automatically. Override with
`ACTION_VM_CART`, `ACTION_VM_OS`, or `ACTIONC_ATARI800_CART` when comparing
against another ROM.
