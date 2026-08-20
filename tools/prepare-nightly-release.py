#!/usr/bin/env python3

import argparse
import hashlib
from pathlib import Path
import shutil
import sys
import tarfile
import zipfile


TARGET_ASSETS = {
    "x86_64-unknown-linux-musl": "actionc-nightly-x86_64-unknown-linux-musl.tar.gz",
    "x86_64-pc-windows-msvc": "actionc-nightly-x86_64-pc-windows-msvc.zip",
    "aarch64-apple-darwin": "actionc-nightly-aarch64-apple-darwin.tar.gz",
    "x86_64-apple-darwin": "actionc-nightly-x86_64-apple-darwin.tar.gz",
}
ACTION_RUNTIME_SOURCE_FILES = {
    "BIGST.ACT",
    "CATCH.ACT",
    "KALROM.ACT",
    "SAMPLE.ACT",
    "SAMPLE2.ACT",
    "ST.ACT",
    "SYS.ACT",
    "SYSALL.ACT",
    "SYSBLK.ACT",
    "SYSGR.ACT",
    "SYSIO.ACT",
    "SYSLIB.ACT",
    "SYSMISC.ACT",
    "SYSSTR.ACT",
}
COMMON_PACKAGE_FILES = {
    "BUILD-INFO.txt",
    "LICENSE",
    "README.md",
    "USAGE.md",
    "docs/ACTIONC_RUN.md",
    "licenses/ACTION-ROM-NOTICE.md",
    "licenses/ACTION-RUNTIME-NOTICE.md",
    "licenses/ALTIRRAOS-LICENSE",
    "licenses/MYDOS-NOTICE.md",
    "licenses/MYDOS-SOURCE-README.md",
    "licenses/MYDOS453.ARC",
    "licenses/ROM-IMAGES.md",
    *(f"licenses/runtime-source/{name}" for name in ACTION_RUNTIME_SOURCE_FILES),
}
ACTION_SOURCE_NAME = "action-3.6-source-0b8bcedb.tar.gz"
ACTION_SOURCE_SHA256 = (
    "fa3466ee7286d8e65a4ca5b0b1db69e4428b15ec93b2119ae68811a30528d824"
)
ACTION_SOURCE_REQUIRED_FILES = {
    "action-3.6-source-0b8bcedb/JAC/build/Make-ACTION.bat",
    "action-3.6-source-0b8bcedb/JAC/build/Make-Settings.bat",
    "action-3.6-source-0b8bcedb/JAC/ref/rom/ACTION-36-ROM-OSS.rom",
    "action-3.6-source-0b8bcedb/JAC/src/ACTION-ROM-OSS-16k.asm",
    "action-3.6-source-0b8bcedb/JAC/src/GPL.txt",
}


class ReleaseError(Exception):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Validate and assemble assets for the moving nightly release"
    )
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--action-source", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--channel", default="nightly")
    return parser.parse_args(argv)


def require_regular_file(path: Path, description: str) -> None:
    if path.is_symlink() or not path.is_file():
        raise ReleaseError(f"missing regular {description}: {path}")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as contents:
        for chunk in iter(lambda: contents.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def expected_package_members(target: str) -> set[str]:
    root = f"actionc-nightly-{target}"
    suffix = ".exe" if target.endswith("windows-msvc") else ""
    relative = set(COMMON_PACKAGE_FILES)
    relative.update(
        f"{executable}{suffix}"
        for executable in ("actionc", "actionc-run", "actionc-emit")
    )
    return {f"{root}/{name}" for name in relative}


def validate_build_info(text: str, target: str, commit: str, channel: str) -> None:
    lines = set(text.splitlines())
    required = {
        "package: actionc",
        f"channel: {channel}",
        f"commit: {commit}",
        f"target: {target}",
    }
    missing = sorted(required - lines)
    if missing:
        raise ReleaseError(
            f"{TARGET_ASSETS[target]} has inconsistent BUILD-INFO.txt: "
            + ", ".join(missing)
        )


def validate_member_inventory(path: Path, names: list[str], target: str) -> None:
    if len(names) != len(set(names)):
        raise ReleaseError(f"{path.name} contains duplicate archive members")
    expected = expected_package_members(target)
    actual = set(names)
    if actual != expected:
        missing = ", ".join(sorted(expected - actual)) or "none"
        unexpected = ", ".join(sorted(actual - expected)) or "none"
        raise ReleaseError(
            f"{path.name} inventory mismatch; missing: {missing}; "
            f"unexpected: {unexpected}"
        )


def validate_zip(path: Path, target: str, commit: str, channel: str) -> None:
    try:
        with zipfile.ZipFile(path) as package:
            if bad_member := package.testzip():
                raise ReleaseError(f"{path.name} has corrupt member {bad_member}")
            infos = package.infolist()
            names = [info.filename for info in infos]
            if any(info.is_dir() for info in infos):
                raise ReleaseError(f"{path.name} contains explicit directory entries")
            validate_member_inventory(path, names, target)
            root = f"actionc-nightly-{target}"
            build_info = package.read(f"{root}/BUILD-INFO.txt").decode("utf-8")
    except (UnicodeDecodeError, zipfile.BadZipFile) as error:
        raise ReleaseError(f"invalid nightly archive {path}: {error}") from error
    validate_build_info(build_info, target, commit, channel)


def validate_tar(path: Path, target: str, commit: str, channel: str) -> None:
    try:
        with tarfile.open(path, "r:gz") as package:
            members = package.getmembers()
            if any(not member.isfile() for member in members):
                raise ReleaseError(f"{path.name} contains non-file archive members")
            names = [member.name for member in members]
            validate_member_inventory(path, names, target)
            root = f"actionc-nightly-{target}"
            extracted = package.extractfile(f"{root}/BUILD-INFO.txt")
            if extracted is None:
                raise ReleaseError(f"{path.name} is missing BUILD-INFO.txt")
            build_info = extracted.read().decode("utf-8")
    except (UnicodeDecodeError, tarfile.TarError) as error:
        raise ReleaseError(f"invalid nightly archive {path}: {error}") from error
    validate_build_info(build_info, target, commit, channel)


def validate_package(path: Path, target: str, commit: str, channel: str) -> None:
    require_regular_file(path, "nightly archive")
    if path.name.endswith(".zip"):
        validate_zip(path, target, commit, channel)
    else:
        validate_tar(path, target, commit, channel)


def validate_action_source(path: Path) -> None:
    require_regular_file(path, "Action! corresponding-source archive")
    actual_hash = sha256(path)
    if actual_hash != ACTION_SOURCE_SHA256:
        raise ReleaseError(
            f"unexpected Action! source SHA-256 for {path}: {actual_hash}"
        )
    try:
        with tarfile.open(path, "r:gz") as source:
            members = source.getmembers()
            if any(not (member.isfile() or member.isdir()) for member in members):
                raise ReleaseError(
                    f"{path.name} contains unsupported corresponding-source members"
                )
            names = {member.name for member in members if member.isfile()}
    except tarfile.TarError as error:
        raise ReleaseError(f"invalid Action! source archive {path}: {error}") from error
    missing = sorted(ACTION_SOURCE_REQUIRED_FILES - names)
    if missing:
        raise ReleaseError(
            f"{path.name} is missing corresponding-source files: " + ", ".join(missing)
        )


def validate_input_inventory(input_dir: Path) -> dict[str, Path]:
    if input_dir.is_symlink() or not input_dir.is_dir():
        raise ReleaseError(f"missing input directory: {input_dir}")
    entries = list(input_dir.iterdir())
    actual = {entry.name for entry in entries}
    expected = set(TARGET_ASSETS.values())
    if actual != expected:
        missing = ", ".join(sorted(expected - actual)) or "none"
        unexpected = ", ".join(sorted(actual - expected)) or "none"
        raise ReleaseError(
            f"nightly input inventory mismatch; missing: {missing}; "
            f"unexpected: {unexpected}"
        )
    return {entry.name: entry for entry in entries}


def verify_checksums(checksum_file: Path, asset_paths: list[Path]) -> None:
    expected = {path.name: sha256(path) for path in asset_paths}
    actual = {}
    for line in checksum_file.read_text(encoding="utf-8").splitlines():
        try:
            digest, name = line.split("  ", 1)
        except ValueError as error:
            raise ReleaseError(f"malformed checksum line: {line!r}") from error
        actual[name] = digest
    if actual != expected:
        raise ReleaseError("generated SHA256SUMS did not verify")


def prepare(args: argparse.Namespace) -> Path:
    input_dir = args.input_dir.resolve()
    inputs = validate_input_inventory(input_dir)
    for target, name in TARGET_ASSETS.items():
        validate_package(inputs[name], target, args.commit, args.channel)

    action_source = args.action_source.resolve()
    validate_action_source(action_source)

    output_dir = args.output_dir.resolve()
    if output_dir.exists():
        if output_dir.is_symlink() or not output_dir.is_dir():
            raise ReleaseError(f"release output is not a directory: {output_dir}")
        if any(output_dir.iterdir()):
            raise ReleaseError(f"release output directory is not empty: {output_dir}")
    else:
        output_dir.mkdir(parents=True)

    output_assets = []
    for name in sorted(TARGET_ASSETS.values()):
        destination = output_dir / name
        shutil.copyfile(inputs[name], destination)
        output_assets.append(destination)

    source_destination = output_dir / ACTION_SOURCE_NAME
    shutil.copyfile(action_source, source_destination)
    output_assets.append(source_destination)

    checksum_file = output_dir / "SHA256SUMS"
    checksum_lines = [
        f"{sha256(path)}  {path.name}\n"
        for path in sorted(output_assets, key=lambda path: path.name)
    ]
    checksum_file.write_text("".join(checksum_lines), encoding="utf-8", newline="\n")
    verify_checksums(checksum_file, output_assets)
    return output_dir


def main(argv: list[str] | None = None) -> int:
    try:
        args = parse_args(sys.argv[1:] if argv is None else argv)
        output_dir = prepare(args)
    except (OSError, ReleaseError) as error:
        print(f"prepare-nightly-release: {error}", file=sys.stderr)
        return 1
    print(output_dir)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
