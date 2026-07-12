# Original TN Symbol Dump

This directory captures the original Action! cartridge compiler's symbol tables
while compiling the archived TN sources extracted from `tn-1.23-stryker.atr`.
It exists so TN layout/name-resolution questions can be answered from stable
repo artifacts instead of rerunning the VM probe.

## Inputs

```text
TN source:  corpora/tn/original/extracted/SRC/TN.ACT.atascii
LIB source: corpora/tn/original/extracted/SRC/LIB.ACT.atascii
cart ROM:   roms/action.rom
OS ROM:     roms/rev02.rom
```

The source files are raw ATASCII with `$9B` line endings. The VM host-file
harness passes non-LF bytes through unchanged, so these are fed to the cartridge
compiler as ATASCII rather than as converted UTF-8 text.

## Artifacts

```text
TN.symbols.json             final decoded global table, plus the active local table at stop
TN.symbol-snapshots.json    local-table snapshots captured at Action!'s segment-end hook
TN.globals.tsv              flat grep-friendly global symbol view
TN.locals.tsv               flat grep-friendly local symbol view grouped by routine
TN.COM                      cartridge compiler output from this VM run
```

The symbol dump contains 158 globals. The snapshot dump contains 104 routine
snapshots, including 69 snapshots with locals.

## Reproduction Command

Run from the `actionc` repo root:

```sh
mkdir -p surveys/tn/original-symbols

cargo run --quiet --manifest-path ../action-compiler-vm/Cargo.toml -- run \
  --cart roms/action.rom \
  --os roms/rev02.rom \
  --hotpatch action-q-input \
  --hotpatch action-headless-getkey \
  --host-file 'TN.ACT:corpora/tn/original/extracted/SRC/TN.ACT.atascii' \
  --host-file 'LIB.ACT:corpora/tn/original/extracted/SRC/LIB.ACT.atascii' \
  --host-output 'TN.COM:surveys/tn/original-symbols/TN.COM' \
  --monitor-key-at-pc '$A2E0' \
  --q-input-at-pc-after '$A2E0:$B2F5:C "H:TN.ACT"\nW "H:TN.COM"\n' \
  --dump-symbols-on-stop surveys/tn/original-symbols/TN.symbols.json \
  --dump-symbol-snapshots-on-stop surveys/tn/original-symbols/TN.symbol-snapshots.json \
  --action-symbol-hooks \
  --max-cycles 80000000 \
  --history 20

jq -r '(["name","scope","class","vtype","address","slot","name_addr","numargs","args"] | @tsv),
       (.globals[] | [.name,.scope,.class,.vtype,(.address // ""),.slot,.name_addr,
                      (.numargs|tostring),(.args|join(","))] | @tsv)' \
  surveys/tn/original-symbols/TN.symbols.json \
  > surveys/tn/original-symbols/TN.globals.tsv

jq -r '(["proc","name","class","vtype","address","slot","name_addr","numargs","args"] | @tsv),
       (.snapshots[] | .proc as $proc | .locals[] |
        [$proc,.name,.class,.vtype,(.address // ""),.slot,.name_addr,
         (.numargs|tostring),(.args|join(","))] | @tsv)' \
  surveys/tn/original-symbols/TN.symbol-snapshots.json \
  > surveys/tn/original-symbols/TN.locals.tsv
```

The VM run stops by hitting the step limit after the compile/write sequence has
completed and the Action! monitor is waiting for further `Q:` input. That stop
reason is expected for this capture.

## Checksums And Load Shape

```text
ba3ffb7c54374dc083818fb3c71b20b2676760769c414a5b3f10bc35294b9c70  TN.ACT.atascii
734b1601b9f5fa5c39a1b216d212abd9416509dc1d26f39acc1ad80dce038838  LIB.ACT.atascii
767256069db32310ad4afdf8efdb3e82d89ee195741daea9321bf6ffb4737d7e  TN.COM
7321e463e9114fa873b76a45c7886cf28e0a190da3d442426bbf5ee10c0e98ec  TN.symbols.json
5712080057b6ca17db7229b65a39bae5b6e95332c0f01f6996858284fac91d4c  TN.symbol-snapshots.json
```

The VM-compiled `TN.COM` is 12,041 bytes:

```text
seg 00: $2C00-$5AFC len 12029
seg 01: $02E2-$02E3 len     2, RUNAD=$5A19
```

The prebuilt `corpora/tn/original/extracted/TN.COM` is 12,127 bytes
(`$2C00-$5B52`, `RUNAD=$5A6F`). Treat this capture as the reproducible original
compiler symbol table for the archived ATASCII sources, not as a claim that the
checked-in prebuilt COM came from this exact compiler/session state.
