# Exec816 component size breakdown

2026-09-24, compiler `351cff84`, frozen optimized Exec `622b139-dirty` workload.
The measured compiler image is after native Add/Sub and constant stores. All 631
routines are assigned once. TASKPOLICY routines are attributed to their defining
source include so console, SIO and DOS policy are counted with their respective
components; the remaining routines are grouped by module. Initialized compiler
data is assigned by exact image symbol/segment extents. Platform assembly and
packager data are carried forward from the original packaged image.

Footprint means code plus initialized data after subtracting the 72,252 measured
compiler stack-guard bytes. This is an estimate for release budgeting, not an
actual build with all guards disabled. A guard-off rebuild may change layout and
platform checks. Stacks, heap capacity, BSS and loader overhead are excluded.

| Component | Code excluding compiler guards | Initialized data | Footprint | Share | Add/Sub saved | Constant stores saved |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| DOS and generic filesystem | 124,990 B | 18 B | 122.1 KiB | 35.5% | 292 B | 2,525 B |
| Exec core | 71,106 B | 84 B | 69.5 KiB | 20.2% | 1,298 B | 781 B |
| Console and windows | 51,930 B | 0 B | 50.7 KiB | 14.7% | 306 B | 781 B |
| Shell | 35,605 B | 660 B | 35.4 KiB | 10.3% | 80 B | 253 B |
| SIO and block I/O | 28,207 B | 0 B | 27.5 KiB | 8.0% | 270 B | 502 B |
| MyDOS filesystem | 26,833 B | 0 B | 26.2 KiB | 7.6% | 719 B | 948 B |
| Platform assembly, tables and build info | 8,300 B | 1,545 B | 9.6 KiB | 2.8% | 0 B | 0 B |
| Inspection utilities | 3,338 B | 0 B | 3.3 KiB | 0.9% | 0 B | 7 B |
| **Total** | **350,309 B** | **2,307 B** | **344.4 KiB** | **100%** | **2,965 B** | **5,797 B** |

Estimated loaded footprint: **352,616 bytes**, versus the **262,144-byte** cap.
Remaining gap: **90,472 bytes (88.4 KiB)**, or 25.7% of the current estimate.
The guarded code/data total is 424,868 bytes.

TASKPOLICY includes 41 console, 19 SIO and 11 DOS routines alongside the kernel
services. A module-only split would overstate Exec core and understate these
features. The attribution below uses exact routine declarations in the frozen
source files. Shared dispatch and generic I/O services remain in Exec core. All
120 frozen input hashes were rechecked.

## Group membership

- **DOS and generic filesystem:** `COOKEDLINE`, `DOS`, `DOSBREAK`, `DOSCALLS`, `DOSCANCEL`, `DOSCLIENT`, `DOSCOOKED`, `DOSOBJECTS`, `DOSRAW`, `DOSSTREAMS`, `DOSWIRE`, `FSABORT`, `FSACTIVE`, `FSBOOT`, `FSDIRECTORY`, `FSHANDLER`, `FSINFO`, `FSINIT`, `FSIO`, `FSLOCKS`, `FSMOUNT`, `FSMUX`, `FSNAMES`, `FSOPERATION`, `FSPACKET`, `FSPORTS`, `FSREGISTRY`, `FSRELATIVE`, `FSWORKER`, `TASKPOLICY/task-dos.inc`.
- **Exec core:** `EXECLISTS`, `EXECMEMORY`, `HEAPCORE`, `HEAPPOLICY`, `IOCORE`, `PORTCORE`, `TASKPOLICY/task-io.inc`, `TASKPOLICY/task-memory.inc`, `TASKPOLICY/task-ports.inc`, `TASKPOLICY/task-signals.inc`, `TASKPOLICY/task-wakes.inc`, `TASKPOLICY/taskpolicy.act`.
- **Console and windows:** `CONSOLECORE`, `CONSOLEDISPLAY`, `CONSOLEDRIVER`, `CONSOLEFOREGROUND`, `CONSOLEINPUT`, `CONSOLEWINDOWS`, `TASKPOLICY/task-console.inc`.
- **Shell:** `SHELLAPP`.
- **SIO and block I/O:** `BLOCKIO`, `BLOCKWIRE`, `SIODRIVER`, `TASKPOLICY/task-sio.inc`.
- **MyDOS filesystem:** `MYDOS`, `MYDOSFILE`, `MYDOSNAMES`.
- **Platform assembly, tables and build info:** `EXECBUILD`, plus original packaged assembly and data tables.
- **Inspection utilities:** `DEVICEINSPECT`, `FSINSPECT`, `TASKINSPECT`.

`COOKEDLINE` belongs to the DOS console-stream adapter here. MyDOS excludes
generic FS services and shared disk/block I/O. Inspection utilities remain
included; their classification does not assume they will be removed. These are
ownership totals, not dependency-closure savings from disabling a feature.

## Largest modules/source slices (guards subtracted; initialized data included)

| Module/source slice | Footprint | Routines |
| --- | ---: | ---: |
| SHELLAPP | 35.4 KiB | 42 |
| TASKPOLICY/taskpolicy.act | 22.2 KiB | 42 |
| TASKPOLICY/task-console.inc | 19.5 KiB | 41 |
| DOSCALLS | 15.2 KiB | 28 |
| MYDOSFILE | 12.7 KiB | 20 |
| MYDOS | 11.2 KiB | 24 |
| TASKPOLICY/task-io.inc | 11.0 KiB | 23 |
| FSINIT | 10.9 KiB | 17 |
| DOSRAW | 10.5 KiB | 17 |
| FSHANDLER | 10.2 KiB | 18 |
| TASKPOLICY/task-sio.inc | 10.1 KiB | 19 |
| BLOCKIO | 10.0 KiB | 19 |
| HEAPCORE | 9.4 KiB | 16 |
| HEAPPOLICY | 9.3 KiB | 19 |
| CONSOLEDRIVER | 8.7 KiB | 13 |

Full byte counts and both optimization deltas:
[CSV](component-module-sizes.csv).

Compiler image SHA-256:
`9a5e4e2635cbcd9dbfa7204d8eb3249234ce55f114806499f1384113bda81f85`. Original
packaged image SHA-256:
`200572a14109579065f3bca897c5e7be489a1f893d755408c05577464c6f41d0`.
