#!/usr/bin/env python3

import argparse
import datetime as dt
import gzip
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile


TARGET_ARCHIVES = {
    "x86_64-unknown-linux-musl": "tar.gz",
    "x86_64-pc-windows-msvc": "zip",
    "aarch64-apple-darwin": "tar.gz",
    "x86_64-apple-darwin": "tar.gz",
}
EXECUTABLES = ("actionc", "actionc-run", "actionc-emit")
ARCHIVE_EXECUTABLE_NAMES = frozenset(
    (*EXECUTABLES, *(f"{executable}.exe" for executable in EXECUTABLES))
)
REQUIRED_LICENSE_FILES = {
    "roms/ACTION-ROM-NOTICE.md": "the embedded Action! cartridge",
    "atr/MYDOS-NOTICE.md": "the embedded MyDOS disk image",
    "atr/source/MYDOS453.ARC": "machine-readable MyDOS 4.53/3 source",
}
ACTION_RUNTIME_SOURCE_FILES = (
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
)


class PackageError(Exception):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Build one actionc nightly archive")
    parser.add_argument("--target", choices=sorted(TARGET_ARCHIVES), required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--commit", default=os.environ.get("ACTIONC_BUILD_SHA"))
    parser.add_argument("--build-date", default=os.environ.get("ACTIONC_BUILD_DATE"))
    parser.add_argument(
        "--channel", default=os.environ.get("ACTIONC_BUILD_CHANNEL", "nightly")
    )
    parser.add_argument("--rustc", default="rustc")
    parser.add_argument(
        "--rustc-version",
        help="record this text instead of invoking rustc --version --verbose",
    )
    parser.add_argument(
        "--allow-incomplete-license-notices",
        action="store_true",
        help="create a prepublication archive even when embedded-asset license material is missing",
    )
    args = parser.parse_args(argv)
    if not args.commit:
        parser.error("--commit or ACTIONC_BUILD_SHA is required")
    if not args.build_date:
        parser.error("--build-date or ACTIONC_BUILD_DATE is required")
    return args


def parse_build_date(value: str) -> tuple[str, int]:
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise PackageError(f"invalid --build-date {value!r}: {error}") from error
    if parsed.tzinfo is None:
        raise PackageError("--build-date must include a UTC offset or Z suffix")
    normalized = parsed.astimezone(dt.timezone.utc)
    return value, int(normalized.timestamp())


def validate_channel(value: str) -> str:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", value):
        raise PackageError(
            "--channel must contain only letters, digits, dots, underscores, and hyphens"
        )
    return value


def require_regular_file(path: Path, description: str) -> None:
    if path.is_symlink() or not path.is_file():
        raise PackageError(f"missing regular {description}: {path}")


def copy_file(source: Path, destination: Path) -> None:
    require_regular_file(source, "package input")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, destination)


def rustc_version(args: argparse.Namespace) -> str:
    if args.rustc_version is not None:
        return args.rustc_version.strip()
    try:
        completed = subprocess.run(
            [args.rustc, "--version", "--verbose"],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise PackageError(
            f"could not query Rust toolchain with {args.rustc!r}: {error}"
        ) from error
    return completed.stdout.strip()


def package_version(repo_root: Path) -> str:
    with (repo_root / "Cargo.toml").open("rb") as cargo_file:
        manifest = tomllib.load(cargo_file)
    try:
        return manifest["package"]["version"]
    except (KeyError, TypeError) as error:
        raise PackageError("Cargo.toml does not contain package.version") from error


def license_inputs(
    repo_root: Path, allow_incomplete: bool
) -> tuple[list[tuple[Path, Path]], list[str]]:
    inputs = [
        (repo_root / "roms/ALTIRRAOS-LICENSE", Path("ALTIRRAOS-LICENSE")),
        (repo_root / "roms/README.md", Path("ROM-IMAGES.md")),
        (
            repo_root / "atr/source/README.md",
            Path("MYDOS-SOURCE-README.md"),
        ),
    ]
    missing = []
    runtime_notice = repo_root / "corpora/action-runtime/README.md"
    if runtime_notice.is_file() and not runtime_notice.is_symlink():
        inputs.append((runtime_notice, Path("ACTION-RUNTIME-NOTICE.md")))
    else:
        missing.append(
            "corpora/action-runtime/README.md: the embedded Action runtime source"
        )
    for file_name in ACTION_RUNTIME_SOURCE_FILES:
        source = repo_root / "corpora/action-runtime/extracted" / file_name
        if source.is_file() and not source.is_symlink():
            inputs.append((source, Path("runtime-source") / file_name))
        else:
            missing.append(
                f"corpora/action-runtime/extracted/{file_name}: "
                "embedded Action runtime corresponding source"
            )
    for relative, description in REQUIRED_LICENSE_FILES.items():
        source = repo_root / relative
        if source.is_file() and not source.is_symlink():
            inputs.append((source, Path(relative).name))
        else:
            missing.append(f"{relative}: {description}")

    if missing and not allow_incomplete:
        joined = "\n  ".join(missing)
        raise PackageError(
            "embedded-asset license material is incomplete:\n"
            f"  {joined}\n"
            "refusing to create a publishable archive; use "
            "--allow-incomplete-license-notices only for prepublication CI"
        )
    return inputs, missing


def build_info_text(
    args: argparse.Namespace, version: str, build_date: str, rustc: str
) -> str:
    rustc_lines = rustc.splitlines() or ["unknown"]
    indented_rustc = "\n".join(f"  {line}" for line in rustc_lines)
    return (
        f"package: actionc\n"
        f"version: {version}\n"
        f"channel: {args.channel}\n"
        f"commit: {args.commit}\n"
        f"build-date: {build_date}\n"
        f"target: {args.target}\n"
        f"rustc:\n{indented_rustc}\n"
    )


def incomplete_notice_text(missing: list[str]) -> str:
    entries = "\n".join(f"- {item}" for item in missing)
    return (
        "# Incomplete Licensing Material\n\n"
        "This archive was produced for prepublication CI only. It must not be\n"
        "attached to a public release until the following embedded-asset\n"
        f"notices, provenance records, and source archives are present:\n\n{entries}\n"
    )


def stage_package(
    args: argparse.Namespace, repo_root: Path, stage_root: Path, build_date: str
) -> None:
    windows = args.target.endswith("windows-msvc")
    suffix = ".exe" if windows else ""
    for executable in EXECUTABLES:
        file_name = f"{executable}{suffix}"
        copy_file(args.bin_dir / file_name, stage_root / file_name)

    for relative in ("README.md", "USAGE.md", "LICENSE"):
        copy_file(repo_root / relative, stage_root / relative)
    copy_file(
        repo_root / "docs/ACTIONC_RUN.md",
        stage_root / "docs/ACTIONC_RUN.md",
    )
    copy_file(
        repo_root / "docs/Action_2027/MODULES_AND_RUNTIME_USAGE.md",
        stage_root / "docs/Action_2027/MODULES_AND_RUNTIME_USAGE.md",
    )

    notices, missing = license_inputs(
        repo_root, args.allow_incomplete_license_notices
    )
    for source, relative in notices:
        copy_file(source, stage_root / "licenses" / relative)
    if missing:
        notice = stage_root / "licenses/INCOMPLETE-LICENSING.md"
        notice.parent.mkdir(parents=True, exist_ok=True)
        notice.write_text(incomplete_notice_text(missing), encoding="utf-8", newline="\n")

    info = build_info_text(
        args, package_version(repo_root), build_date, rustc_version(args)
    )
    (stage_root / "BUILD-INFO.txt").write_text(info, encoding="utf-8", newline="\n")


def staged_files(stage_root: Path) -> list[Path]:
    files = []
    for path in stage_root.rglob("*"):
        if path.is_symlink():
            raise PackageError(f"refusing to archive symbolic link: {path}")
        if path.is_file():
            files.append(path)
    return sorted(files, key=lambda path: path.relative_to(stage_root).as_posix())


def archive_mode(stage_root: Path, source: Path) -> int:
    relative = source.relative_to(stage_root)
    if len(relative.parts) == 1 and relative.name in ARCHIVE_EXECUTABLE_NAMES:
        return 0o755
    return 0o644


def write_tar_gz(stage_root: Path, archive: Path, timestamp: int) -> None:
    root_name = stage_root.name
    with archive.open("wb") as raw_file:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw_file, mtime=timestamp) as gzip_file:
            with tarfile.open(fileobj=gzip_file, mode="w", format=tarfile.PAX_FORMAT) as tar:
                for source in staged_files(stage_root):
                    relative = source.relative_to(stage_root).as_posix()
                    info = tar.gettarinfo(str(source), arcname=f"{root_name}/{relative}")
                    info.uid = 0
                    info.gid = 0
                    info.uname = "root"
                    info.gname = "root"
                    info.mtime = timestamp
                    info.mode = archive_mode(stage_root, source)
                    with source.open("rb") as contents:
                        tar.addfile(info, contents)


def write_zip(stage_root: Path, archive: Path, timestamp: int) -> None:
    root_name = stage_root.name
    date_time = dt.datetime.fromtimestamp(max(timestamp, 315532800), dt.timezone.utc)
    zip_time = (
        date_time.year,
        date_time.month,
        date_time.day,
        date_time.hour,
        date_time.minute,
        date_time.second,
    )
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as output:
        for source in staged_files(stage_root):
            relative = source.relative_to(stage_root).as_posix()
            info = zipfile.ZipInfo(f"{root_name}/{relative}", date_time=zip_time)
            info.compress_type = zipfile.ZIP_DEFLATED
            info.create_system = 3
            mode = archive_mode(stage_root, source)
            info.external_attr = (stat.S_IFREG | mode) << 16
            output.writestr(info, source.read_bytes())


def package(args: argparse.Namespace) -> Path:
    repo_root = Path(__file__).resolve().parents[1]
    build_date, timestamp = parse_build_date(args.build_date)
    args.channel = validate_channel(args.channel)
    license_inputs(repo_root, args.allow_incomplete_license_notices)
    args.bin_dir = args.bin_dir.resolve()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output_dir = args.output_dir.resolve()
    archive_kind = TARGET_ARCHIVES[args.target]
    root_name = f"actionc-{args.channel}-{args.target}"
    archive_name = f"{root_name}.{archive_kind}"
    destination = output_dir / archive_name

    with tempfile.TemporaryDirectory(
        prefix="actionc-package-", dir=output_dir
    ) as temporary:
        temporary_path = Path(temporary)
        stage_root = temporary_path / root_name
        stage_root.mkdir()
        stage_package(args, repo_root, stage_root, build_date)
        temporary_archive = temporary_path / archive_name
        if archive_kind == "zip":
            write_zip(stage_root, temporary_archive, timestamp)
        else:
            write_tar_gz(stage_root, temporary_archive, timestamp)
        os.replace(temporary_archive, destination)

    return destination


def main(argv: list[str] | None = None) -> int:
    try:
        args = parse_args(sys.argv[1:] if argv is None else argv)
        archive = package(args)
    except (OSError, PackageError) as error:
        print(f"package-nightly: {error}", file=sys.stderr)
        return 1
    print(archive)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
