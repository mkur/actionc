#!/usr/bin/env python3
"""Build an Amiga smoke bundle and verify files copied back from its Shell run."""
import argparse
import hashlib
import json
import re
import subprocess
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SAMPLES = ("hello", "integer-array", "insertsort", "division-zero")


def expected(name):
    # References are host text; executable and returned console bytes are binary.
    return (ROOT / "samples/amiga/expected" / f"{name}.txt").read_text().encode("ascii")


def runs():
    for name in SAMPLES:
        for mode in ("console", "redirect"):
            yield f"{name}.{mode}", name, mode, 20 if name == "division-zero" else 0
    yield "hello.recovery", "hello", "console", 0


def script(run_id):
    # Classic Shell RC expansion and FAILAT/IF/QUIT contracts:
    # https://wiki.amigaos.net/wiki/AmigaOS_Manual:_AmigaDOS_Environment_Variables
    # https://wiki.amigaos.net/wiki/AmigaOS_Manual:_AmigaDOS_Command_Reference
    # Unbraced Shell variable references accept only letters and digits.
    # Keep actioncrc alphanumeric: $actionc_rc would expand $actionc instead.
    # RKRM AmigaDOS (2024), section 15.1.5:
    # https://developer.amigaos3.net/sites/default/files/downloads/2024-10/Amiga_ROM_Kernel_Reference_Manual_DOS.pdf
    lines = [
        "; Run with Execute smoke from this directory in an AmigaOS 3.1 Shell.",
        "Stack 65536",
        "FailAt 21",
        f'Echo "BUILD {run_id}" >RAM:actionc-status.txt',
        "Version exec.library >RAM:actionc-exec-version.txt",
        "Version dos.library >RAM:actionc-dos-version.txt",
    ]
    for label, name, mode, status in runs():
        lines.append(f'Echo "BEGIN {label}"')
        redirect = f" >RAM:actionc-{name}.txt" if mode == "redirect" else ""
        lines.extend([
            f"{name}.amiga{redirect}",
            "Set actioncrc $RC",  # Capture immediately, before another command.
            f'Echo "RC {label} $actioncrc" >>RAM:actionc-status.txt',
            f"If NOT $actioncrc EQ {status} VAL",
            f'Echo "FAIL {label}" >>RAM:actionc-status.txt',
            f'Echo "FAIL {label}: return code $actioncrc, expected {status}"',
            "Quit 20",
            "EndIf",
        ])
        if mode == "redirect":
            lines.append(f"Type RAM:actionc-{name}.txt")
    lines.extend(['Echo "SMOKE PASS" >>RAM:actionc-status.txt', "Type RAM:actionc-status.txt", "Quit 0"])
    return ("\n".join(lines) + "\n").encode("ascii")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(compiler, output):
    compiler = compiler.resolve(strict=True)
    output.mkdir(parents=True, exist_ok=True)
    run_id = uuid.uuid4().hex
    files = {}
    for name in SAMPLES:
        path = output / f"{name}.amiga"
        subprocess.run([str(compiler), "--target", "motorola-68000", "--runtime", "amiga", "-o", str(path), str(ROOT / "samples/amiga" / f"{name}.act")], check=True)
        files[path.name] = digest(path)
    smoke = output / "smoke"
    smoke.write_bytes(script(run_id))
    files[smoke.name] = digest(smoke)
    reference = output / "expected"
    reference.mkdir(exist_ok=True)
    for name in SAMPLES:
        path = reference / f"{name}.txt"
        path.write_bytes(expected(name))
        files[f"expected/{name}.txt"] = digest(path)
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    version = subprocess.check_output([str(compiler), "--version"], text=True).strip()
    manifest = {"version": 1, "run_id": run_id, "revision": revision, "compiler": version, "compiler_sha256": digest(compiler), "files": files}
    (output / "manifest.json").write_bytes((json.dumps(manifest, indent=2) + "\n").encode("ascii"))
    return manifest


def verify(bundle, collected):
    manifest = json.loads((bundle / "manifest.json").read_text())
    if manifest.get("version") != 1 or not re.fullmatch(r"[0-9a-f]{32}", manifest.get("run_id", "")):
        raise ValueError("invalid smoke manifest")
    required = {"smoke", *(f"{name}.amiga" for name in SAMPLES), *(f"expected/{name}.txt" for name in SAMPLES)}
    if set(manifest["files"]) != required:
        raise ValueError("smoke bundle file inventory mismatch")
    for name, sha in manifest["files"].items():
        if digest(bundle / name) != sha:
            raise ValueError(f"bundle changed after build: {name}")
    status = (collected / "actionc-status.txt").read_bytes()
    if b"$" in status:
        raise ValueError("unexpanded Shell variable in status report; rerun a regenerated smoke script")
    wanted = [f"BUILD {manifest['run_id']}", *(f"RC {label} {rc}" for label, _, _, rc in runs()), "SMOKE PASS"]
    if status != ("\n".join(wanted) + "\n").encode("ascii"):
        raise ValueError("missing success marker, wrong run ID or unexpected Shell status")
    for name in SAMPLES:
        if (collected / f"actionc-{name}.txt").read_bytes() != (bundle / "expected" / f"{name}.txt").read_bytes():
            raise ValueError(f"Amiga output mismatch: {name}")
    for library in ("exec", "dos"):
        version = (collected / f"actionc-{library}-version.txt").read_bytes()
        if not re.search(rb"\b" + library.encode() + rb"\.library\s+40\.[0-9]+\b", version, re.IGNORECASE):
            raise ValueError(f"expected AmigaOS 3.1 {library}.library version 40")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generate = commands.add_parser("build", help="compile samples and write the Shell script")
    generate.add_argument("--compiler", type=Path, required=True)
    generate.add_argument("--output", type=Path, default=ROOT / "build/amiga-smoke")
    check = commands.add_parser("verify", help="check the actual files copied back from RAM:")
    check.add_argument("--bundle", type=Path, default=ROOT / "build/amiga-smoke")
    check.add_argument("collected", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "build":
            manifest = build(args.compiler, args.output)
            print(f"Bundle ready: {args.output}\nRun ID: {manifest['run_id']}")
        else:
            manifest = verify(args.bundle, args.collected)
            print(f"AmigaOS smoke output/status checks passed for {manifest['revision']}")
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"amiga smoke: {error}\n")


if __name__ == "__main__":
    main()
