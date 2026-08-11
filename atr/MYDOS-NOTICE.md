# MyDOS Notice and Corresponding Source

## Bundled image

`mydos.atr` is a configured MyDOS 4.53/3 boot disk embedded in `actionc-run`.
MyDOS was originally written by Charles Marslett and Robert Puff. Version
4.53/3 contains modifications by David R. Eichel and was released on January
1, 1990.

The bundled image has this identity:

```text
size:    183952 bytes
SHA-256: 88c0c6cf080fce8d356d1108c0c78747fff7a4eb6520455e835fe8a17697bbc9
format:  ATR, 720 sectors, 256 bytes per sector
files:   DOS.SYS and DUP.SYS
```

The embedded files have these identities:

```text
DOS.SYS  4554 bytes  SHA-256 4870df6f6f559535438b6f3c5ee8e93b0e7ac7934fe508dba6471bbfc384fa95
DUP.SYS  6708 bytes  SHA-256 28ed7d93b3ac3f388bb357ba98a9c18754e97dc75f61a0380830c74109541817
```

The identifiable configuration and behavior are:

- drives 1 and 2 configured as double-density disk drives;
- drive 8 configured as a RAM disk;
- drive 1 selected as the default drive;
- three-character sector counts, the 4.53/3 default;
- `.AR0` through `.AR9` boot-file support provided by MyDOS 4.53/3.

## Copyright and distribution terms

The following notice is transcribed from page 1 of the *MYDOS Version 4 User
Guide, Revision 4.50*. Its wording, including the apparent `and restriction`
typo in condition 3, is preserved as printed.

The scan used for the transcription is available from:

```text
https://atariwiki.org/wiki/attach/MyDOS/MYDOS_Version_4.50_User_Guide.pdf
size:    78142 bytes
SHA-256: e7f1b02b8d3cf3d74de852e7d9d017ace361209dd2cf85d664fe5e003dc4be65
```

```text
MYDOS Version 4 User Guide
Revision 4.50
for Atari Home Computers

Copyright (C) 1988 by WORDMARK Systems and the authors:

Charles Marslett
2705 Pinewood Dr.
Garland, TX 75042
CIS: 73317,3662
UseNet: CHASM@KILLER.DALLAS.TX.US

and

Robert Puff
Suite 222
2117 Buffalo Rd.
Rochester, NY 14624
GEnie: BOB.PUFF

This software may be freely used and distributed provided that
this copyright notice is left intact, and provided that:

(1) The source code in machine readable form is provided with
any binary distribution, or made available at no additional cost
to the recipients of the binary distribution.

(2) A binary version of a derivative work may be sold for a
reasonable distribution charge (less than $50), and the
source code in machine readable format must be available.

(3) A derivative work may not impose and restriction on the free
distribution of the source code.
```

These are MyDOS's own historical distribution terms. MyDOS is not relicensed
under actionc's GNU GPL license.

## Version 4.53/3 source and attribution

The corresponding 4.53/3 source release is preserved unchanged as
`source/MYDOS453.ARC`. It was obtained from:

```text
https://www.mathyvannisselroy.nl/mydos453.arc
```

The preserved archive has this identity:

```text
size:    87522 bytes
SHA-256: 52853bdf6fa03c73cf1292c9ec6ca355f8109056d71a7531b05b51a4fdb75e87
```

Its `CONTENTS.` file identifies the original program as the work of Charles
Marslett and Bob Puff, attributes the 4.53/3 modifications to David R. Eichel,
and gives the release date as January 1, 1990. The archive contains the complete
MAC/65 source in `MDOS*.M65` and `MDUP*.M65`; its `READ.ME` states that the DOS
and DUP code is complete and ready to assemble using MAC/65.

The source archive is included in actionc source distributions and copied into
every actionc binary package that contains `actionc-run`. Recipients therefore
receive machine-readable MyDOS source with the embedded binary, without relying
on continued availability of the upstream site.

## Relationship to the upstream release disk

`MYDOS453.ARC` also contains `MD453_3.DSK`, a Disk Communicator image of the
released MyDOS 4.53/3 disk. Converting it to ATR produces a 720-sector,
128-byte-sector disk. The actionc image is instead a 720-sector,
256-byte-sector disk saved with the configuration listed above.

MyDOS stores its live drive and memory configuration in `DOS.SYS` and
`DUP.SYS`. The user guide documents menu command `H` as writing both files and
states that the files reflect the configuration currently in memory. The
bundled files are therefore configuration products rather than byte-for-byte
copies of the single-density reference disk:

- `DOS.SYS` matches the reference through its first 4059 bytes; subsequent
  differences are in the saved configuration/tail area and sector padding;
- `DUP.SYS` has the same 6708-byte length and differs at 13 byte positions,
  including the displayed drive configuration;
- the bundled ATR contains only the newly written `DOS.SYS` and `DUP.SYS`.

To produce an equivalent configured disk from the preserved release:

1. Extract `MYDOS453.ARC` and convert `MD453_3.DSK` from Disk Communicator
   format to ATR.
2. Boot that release and initialize a 720-sector, 256-byte-sector target disk.
3. Configure drives 1 and 2 for double density, drive 8 as the RAM disk, and
   drive 1 as the default.
4. Use MyDOS command `H` to write the configured `DOS.SYS` and `DUP.SYS` to the
   target disk, and retain those two files on the template.

This documents the source and configuration relationship. It does not claim
that the historical configuration session has been reproduced byte-for-byte;
the hashes above remain the canonical identity of the embedded image.
