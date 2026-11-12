#!/usr/bin/env python3
"""Compare isolated TN window-frame implementations using the current compiler."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from concurrent.futures import ThreadPoolExecutor

ROOT = Path(__file__).resolve().parents[3]
BUILD = ROOT / "build/tn-window-frames"
VM = ROOT / "tools/vm-runtime-tests"
HERE = Path(__file__).resolve().parent

LINKED_OPEN = """  CARD buf, count
 WindowFrame POINTER saved
 BYTE h

  count=(dx+1)*(dy+1)
  saved=WindowFrame POINTER(Alloc(count+10))
  saved.previous=CARD(windowTop)
  MovePage(CARD(saved)+2, @zx, 4)
  windowTop=saved
  openwindows==+1
  zx=x zy=y zdx=dx zdy=dy
  Relpos(0,0)
  buf=CARD(saved)+10
  GetImage(buf,zdx,zdy)
  MovePage(CARD(saved)+6, @zx, 4)

"""

LINKED_CLOSE = """PROC CloseWindow()
 CARD cursor, mark, previous

  cursor=ArenaCursor()
  IF cursor-WINDOW_ARENA<10 THEN ArenaStop(ARENA_UNDERFLOW) FI
  mark=CARD(windowTop)
  IF mark<WINDOW_ARENA THEN ArenaStop(ARENA_BAD_MARK) FI
  IF mark>cursor-10 THEN ArenaStop(ARENA_BAD_MARK) FI
  previous=windowTop.previous
  MovePage(@zx,mark+6,4)
  openwindows==-1
  Relpos(0,0)
  PutImage(mark+10,zdx,zdy)
  MovePage(@zx,mark+2,4)
  allocp=CARD POINTER(mark)
  windowTop=WindowFrame POINTER(previous)
RETURN
"""

TRAILER_OPEN = """  CARD mark, buf, count
 CARD POINTER imageSlot
 BYTE h

  count=(dx+1)*(dy+1)
  mark=Alloc(count+10)
  MovePage(mark,@zx,4)
  openwindows==+1
  zx=x zy=y zdx=dx zdy=dy
  Relpos(0,0)
  buf=mark+4
  GetImage(buf,zdx,zdy)
  imageSlot=CARD POINTER(buf+count)
  imageSlot^=buf
  MovePage(CARD(imageSlot)+2,@zx,4)

"""

TRAILER_CLOSE = """PROC CloseWindow()
 CARD cursor, buf
 CARD POINTER imageSlot

  cursor=ArenaCursor()
  IF cursor-WINDOW_ARENA<10 THEN ArenaStop(ARENA_UNDERFLOW) FI
  imageSlot=CARD POINTER(cursor-6)
  buf=imageSlot^
  IF buf<WINDOW_ARENA+4 THEN ArenaStop(ARENA_BAD_MARK) FI
  IF buf>cursor-6 THEN ArenaStop(ARENA_BAD_MARK) FI
  MovePage(@zx,cursor-4,4)
  openwindows==-1
  Relpos(0,0)
  PutImage(buf,zdx,zdy)
  MovePage(@zx,buf-4,4)
  allocp=CARD POINTER(buf-4)
RETURN
"""


def read(path):
    return path.read_text().replace("\r\n", "\n")


def replace_once(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new)


def sources():
    lib = read(ROOT / "samples/tn/modern/LIB.ACT")
    for variant in ["baseline", "linked", "trailer"]:
        text = lib
        if variant != "baseline":
            start = text.index("PROC Window(BYTE x, y, dx, dy)")
            body = text.index("\n", start) + 1
            drawing = text.index("  Block(CLEAR, zdx, zdy)", body)
            text = text[:body] + (LINKED_OPEN if variant == "linked" else TRAILER_OPEN) + text[drawing:]
            start = text.index("PROC CloseWindow()")
            end = text.index("\n\nCHAR FUNC Range(", start)
            text = text[:start] + (LINKED_CLOSE if variant == "linked" else TRAILER_CLOSE) + text[end:]
            if variant == "linked":
                text = replace_once(text, " BYTE openwindows\n", """ BYTE openwindows
 TYPE WindowFrame=[CARD previous BYTE parentX,parentY,parentDX,parentDY,x,y,dx,dy]
 WindowFrame POINTER windowTop=[0]
""")
                text = replace_once(text, "  openwindows = 0\n", "  openwindows = 0\n  windowTop=WindowFrame POINTER(0)\n")
        folder = BUILD / variant
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "LIB.ACT").write_bytes(text.encode())
        for name in ["TN.ACT", "TNDBG.ACT"]:
            (folder / name).write_bytes(read(ROOT / "samples/tn/modern" / name).encode())


def prepare_probe():
    subprocess.run(["cargo", "test", "--release", "--locked", "--manifest-path", str(VM / "Cargo.toml"),
                    "--test", "tn_checked_arena", "--no-run"], cwd=ROOT,
                   env=dict(os.environ, CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2"), check=True)
    # Reuse the maintained VM oracles without editing production sources/tests.
    for name in ["tn_checked_arena", "tn_dispatch"]:
        text = read(VM / "tests" / f"{name}.rs")
        text = re.sub(r'#\[path = "support/([^\"]+)"\]',
                      lambda m: '#[path = ' + json.dumps(str(VM / "tests/support" / m[1])) + ']', text)
        if name == "tn_checked_arena":
            text = replace_once(text, 'root().join("samples/tn/modern").join(name)',
                                'PathBuf::from(std::env::var_os("ACTIONC_TN_FRAME_SOURCES").unwrap()).join(name)')
            # Bind both newline variants and both runtimes to measured images.
            marker = '                    let (bytes, listing) = compile(&sources, name, mode, runtime);'
            text = replace_once(text, marker, marker + "\n" + '''                    let stem = format!("{}-{mode:?}-{runtime:?}", name.trim_end_matches(".ACT"));
                    let measured=PathBuf::from(std::env::var_os("ACTIONC_TN_FRAME_SOURCES").unwrap()).join(format!("{stem}.xex"));
                    assert_eq!(bytes,std::fs::read(measured).unwrap());''')
        else:
            text = replace_once(text, 'root().join("samples/tn/modern").join(source)',
                                'PathBuf::from(std::env::var_os("ACTIONC_TN_FRAME_SOURCES").unwrap()).join(source)')
            marker = '            let listing = compiled.source_listing();'
            text = replace_once(text, marker, '''            let measured=PathBuf::from(std::env::var_os("ACTIONC_TN_FRAME_SOURCES").unwrap()).join(format!("{}-{mode:?}-ActionCart.xex",source.trim_end_matches(".ACT")));
            assert_eq!(compiled.object_bytes(),std::fs::read(measured).unwrap());
''' + marker)
        (BUILD / f"{name}.rs").write_text(text)
    wrapper = ''.join(f'#[path={json.dumps(str(path))}] mod {name};\n' for name, path in [
        ("experiment", HERE / "window_frames_probe.rs"),
        ("checked", BUILD / "tn_checked_arena.rs"),
        ("dispatch", BUILD / "tn_dispatch.rs")])
    (BUILD / "probe.rs").write_text(wrapper)
    deps = VM / "target/release/deps"
    cmd = ["rustc", "--edition=2024", "--test", "-O", "--crate-name", "tn_window_frames",
           str(BUILD / "probe.rs"), "-L", f"dependency={deps}", "-o", str(BUILD / "probe")]
    for crate in ["actionc", "actionc_vm", "serde_json"]:
        paths = list(deps.glob(f"lib{crate}-*.rlib"))
        assert len(paths) == 1, f"Build the current VM package in release mode; ambiguous/missing {crate}: {paths}"
        cmd += ["--extern", f"{crate}={paths[0]}"]
    env = dict(os.environ, CARGO_MANIFEST_DIR=str(VM), ACTIONC_TN_FRAME_BUILD=str(BUILD))
    subprocess.run(cmd, cwd=ROOT, env=env, check=True)
    return env


def scorecard(compiler_revision, source_hashes):
    images = json.loads((BUILD / "images.json").read_text())
    execution = json.loads((BUILD / "execution.json").read_text())
    variants = ["baseline", "linked", "trailer"]
    rows = []
    for base in [r for r in images if r["variant"] == "baseline"]:
        key = {k: base[k] for k in ["program", "mode", "runtime"]}
        same = lambda row: all(row[k] == v for k, v in key.items())
        versions = {v: next(r for r in images if same(r) and r["variant"] == v) for v in variants}
        names = {"Window", "CloseWindow", "Init"}
        for version in versions.values():
            names.update(n for n, size in version["routine_bytes"].items() if base["routine_bytes"].get(n) != size)
        cases = {}
        for case in ["small", "nested", "debug", "full-screen", "exact-fit", "one-byte-short"]:
            records = {v: next(r for r in execution if same(r) and r["variant"] == v and r["case"] == case) for v in variants}
            if case == "one-byte-short":
                assert all(r["fault"] == 2 for r in records.values())
                cases[case] = {"fault": 2, "cursor_after": {v: r["cursor_after"] for v, r in records.items()}}
            else:
                peak = records["baseline"]["peak"]
                assert all(r["peak"] == peak for r in records.values())
                cases[case] = {"peak": peak}
                for op in ["open", "close"]:
                    cases[case][f"{op}_cycles"] = {v: [m["cycles"] for m in r["measurements"] if m["op"] == op] for v, r in records.items()}
        image_hashes = {v: hashlib.sha256((BUILD / v / f'{key["program"]}-{key["mode"]}-{key["runtime"]}.xex').read_bytes()).hexdigest() for v in variants}
        rows.append({**key, "xex_bytes": {v: r["xex_bytes"] for v, r in versions.items()},
                     "image_sha256": image_hashes,
                     "routine_bytes": {n: {v: r["routine_bytes"].get(n) for v, r in versions.items()} for n in sorted(names)},
                     "cases": cases})
    return dict(schema=1, compiler_revision=compiler_revision, source_sha256=source_hashes,
                window_overhead_bytes=10, arena_cursor_calls_per_pair=dict(baseline=8, linked=2, trailer=2),
                ranges_preserved=[0x2000, 0x2800], measurements=rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ui", action="store_true", help="also run maintained arena and full UI oracles")
    parser.add_argument("--check", action="store_true", help="compare with recorded experimental results")
    parser.add_argument("--reuse-images", action="store_true", help="reuse images from unchanged sources and committed compiler")
    parser.add_argument("--mads", action="store_true", help="reassemble all images with LF and CRLF listings")
    args = parser.parse_args()
    BUILD.mkdir(parents=True, exist_ok=True)
    sources()
    env = prepare_probe()
    path = ROOT / "docs/benchmarks/6502-tn-window-frames/results.json"
    source_hashes = {f"{v}/{name}": hashlib.sha256((BUILD / v / name).read_bytes()).hexdigest()
                     for v in ["baseline", "linked", "trailer"] for name in ["TN.ACT", "TNDBG.ACT", "LIB.ACT"]}
    compiler_revision = subprocess.check_output(["git", "log", "-1", "--format=%H", "--", "src", "include", "Cargo.lock"], cwd=ROOT, text=True).strip()
    if args.reuse_images:
        previous = json.loads(path.read_text())
        assert previous["source_sha256"] == source_hashes
        assert previous["compiler_revision"] == compiler_revision
        assert not subprocess.check_output(["git", "diff", "HEAD", "--", "src", "include", "Cargo.lock"], cwd=ROOT), "Cannot reuse images with compiler edits"
    for test in ["compile_images", "measure_frames"]:
        if test == "compile_images" and args.reuse_images:
            continue
        with (BUILD / f"{test}.log").open("w") as out:
            subprocess.run([str(BUILD / "probe"), "--exact", f"experiment::{test}", "--nocapture"],
                           env=env, cwd=ROOT, stdout=out, stderr=subprocess.STDOUT, check=True)
    if args.ui:
        def check_ui(variant):
            for test in ["checked::real_arena_success_and_window_checkpoints_with_lf_and_crlf",
                         "dispatch::tn_dispatch_and_panel_state_preserve_behavior"]:
                with (BUILD / f"{variant}-{test.split('::')[0]}.log").open("w") as out:
                    subprocess.run([str(BUILD / "probe"), "--exact", test, "--nocapture"], cwd=ROOT,
                                   env=dict(env, ACTIONC_TN_FRAME_SOURCES=str(BUILD / variant)),
                                   stdout=out, stderr=subprocess.STDOUT, check=True)
        with ThreadPoolExecutor(max_workers=3) as executor:
            list(executor.map(check_ui, ["baseline", "linked", "trailer"]))
    if args.mads:
        count = 0
        for variant in ["baseline", "linked", "trailer"]:
            for asm in sorted((BUILD / variant).glob("*.asm")):
                if ".roundtrip." in asm.name:
                    continue
                listing = read(asm)
                for crlf in [False, True]:
                    source = asm.with_suffix(".roundtrip.asm")
                    image = asm.with_suffix(".roundtrip.xex")
                    source.write_bytes((listing.replace("\n", "\r\n") if crlf else listing).encode())
                    process = subprocess.run([os.environ.get("ACTIONC_MADS", "mads"), str(source), f"-o:{image}", "-s"], capture_output=True, text=True)
                    assert process.returncode == 0, process.stdout + process.stderr
                    assert image.read_bytes() == asm.with_suffix(".xex").read_bytes()
                    source.unlink(); image.unlink()
                    count += 1
        assert count == 48, count
        print(f"MADS: {count} identical LF/CRLF roundtrips")
    result = scorecard(compiler_revision, source_hashes)
    if args.check:
        assert result == json.loads(path.read_text()), "Window frame experiment changed"
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Window frame experiment {'checked' if args.check else 'recorded'}: {path.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
