#!/usr/bin/env python3

import argparse
from pathlib import Path
import subprocess
import sys
import tempfile


EXECUTABLES = ("actionc", "actionc-run", "actionc-emit")


class SmokeError(Exception):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Smoke-test built actionc executables")
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--expect-channel")
    parser.add_argument("--expect-commit")
    parser.add_argument("--all-modes", action="store_true")
    return parser.parse_args(argv)


def run(command: list[Path | str]) -> subprocess.CompletedProcess[bytes]:
    try:
        completed = subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        raise SmokeError(f"could not run {command[0]}: {error}") from error
    if completed.returncode != 0:
        rendered = " ".join(str(part) for part in command)
        raise SmokeError(
            f"command failed ({completed.returncode}): {rendered}\n"
            f"stdout:\n{completed.stdout.decode(errors='replace')}\n"
            f"stderr:\n{completed.stderr.decode(errors='replace')}"
        )
    return completed


def require_load_file(path: Path) -> None:
    contents = path.read_bytes()
    if len(contents) < 6 or contents[:2] != b"\xff\xff":
        raise SmokeError(f"compiler output is not an Atari load file: {path}")


def require_atr(path: Path) -> None:
    contents = path.read_bytes()
    if len(contents) < 16 or contents[:2] != b"\x96\x02":
        raise SmokeError(f"runner output is not an ATR image: {path}")


def smoke(args: argparse.Namespace) -> None:
    bin_dir = args.bin_dir.resolve()
    windows = args.target.endswith("windows-msvc")
    suffix = ".exe" if windows else ""
    binaries = {name: bin_dir / f"{name}{suffix}" for name in EXECUTABLES}
    for name, binary in binaries.items():
        if binary.is_symlink() or not binary.is_file():
            raise SmokeError(f"missing regular {name} executable: {binary}")
        output = run([binary, "--version"]).stdout.decode(errors="replace").strip()
        for expected in (args.expect_channel, args.expect_commit, args.target):
            if expected and expected not in output:
                raise SmokeError(
                    f"{name} version output does not contain {expected!r}: {output!r}"
                )

    repo_root = Path(__file__).resolve().parents[1]
    source = repo_root / "samples/hello-world.act"
    if not source.is_file():
        raise SmokeError(f"missing smoke-test source: {source}")

    modes = (
        ("compatibility", "optimized", "mir6502")
        if args.all_modes
        else ("compatibility",)
    )
    with tempfile.TemporaryDirectory(prefix="actionc-nightly-smoke-") as temporary:
        output_dir = Path(temporary)
        for mode in modes:
            object_file = output_dir / f"hello-{mode}.com"
            run(
                [
                    binaries["actionc"],
                    "--mode",
                    mode,
                    "--output",
                    object_file,
                    source,
                ]
            )
            require_load_file(object_file)

        atr_file = output_dir / "hello.atr"
        run(
            [
                binaries["actionc-run"],
                "--no-run",
                "--out-atr",
                atr_file,
                source,
            ]
        )
        require_atr(atr_file)

        nir = run([binaries["actionc-emit"], "--emit-nir", source]).stdout
        if not nir.startswith(b"nir program\n"):
            raise SmokeError("actionc-emit did not produce NIR text")


def main(argv: list[str] | None = None) -> int:
    try:
        smoke(parse_args(sys.argv[1:] if argv is None else argv))
    except (OSError, SmokeError) as error:
        print(f"smoke-nightly: {error}", file=sys.stderr)
        return 1
    print("nightly smoke tests passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
