#!/usr/bin/env python3

import hashlib
import os
from pathlib import Path
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
import zipfile


REPO_ROOT = Path(__file__).resolve().parents[2]
PACKAGER = REPO_ROOT / "tools/package-nightly.py"
COMMON_FILES = {
    "BUILD-INFO.txt",
    "LICENSE",
    "README.md",
    "USAGE.md",
    "docs/ACTIONC_RUN.md",
    "licenses/ACTION-ROM-NOTICE.md",
    "licenses/ALTIRRAOS-LICENSE",
    "licenses/MYDOS-NOTICE.md",
    "licenses/MYDOS-SOURCE-README.md",
    "licenses/MYDOS453.ARC",
    "licenses/ROM-IMAGES.md",
}


class NightlyPackageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="actionc-package-test-")
        # GitHub's Windows runners keep the checkout on D: and the system
        # temporary directory on C:. Keep package output on the checkout drive
        # so these tests exercise that cross-drive layout.
        self.output_temporary = tempfile.TemporaryDirectory(
            prefix=".actionc-package-output-", dir=REPO_ROOT
        )
        self.root = Path(self.temporary.name)
        self.bin_dir = self.root / "bin"
        self.output_dir = Path(self.output_temporary.name)
        self.bin_dir.mkdir()

    def tearDown(self) -> None:
        self.output_temporary.cleanup()
        self.temporary.cleanup()

    def write_executables(self, suffix: str) -> None:
        for name in ("actionc", "actionc-run", "actionc-emit"):
            path = self.bin_dir / f"{name}{suffix}"
            path.write_bytes(f"fake {name}\n".encode())

    def run_packager(
        self, target: str, *extra: str, metadata_from_environment: bool = False
    ) -> subprocess.CompletedProcess[str]:
        command = [
            sys.executable,
            str(PACKAGER),
            "--target",
            target,
            "--bin-dir",
            str(self.bin_dir),
            "--output-dir",
            str(self.output_dir),
            "--rustc-version",
            "rustc 1.95.0 (package test)",
        ]
        environment = None
        if metadata_from_environment:
            environment = os.environ.copy()
            environment.update(
                ACTIONC_BUILD_SHA="environment-commit",
                ACTIONC_BUILD_DATE="2026-08-12T03:17:00Z",
                ACTIONC_BUILD_CHANNEL="nightly",
            )
        else:
            command.extend(
                [
                    "--commit",
                    "0123456789abcdef",
                    "--build-date",
                    "2026-08-11T03:17:00Z",
                ]
            )
        command.extend(extra)
        return subprocess.run(
            command,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            env=environment,
        )

    def test_linux_archive_has_exact_inventory_and_executable_modes(self) -> None:
        self.write_executables("")
        result = self.run_packager("x86_64-unknown-linux-musl")
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.output_dir / "actionc-nightly-x86_64-unknown-linux-musl.tar.gz"
        self.assertEqual(Path(result.stdout.strip()), archive.resolve())

        root = "actionc-nightly-x86_64-unknown-linux-musl"
        with tarfile.open(archive, "r:gz") as package:
            files = {member.name: member for member in package if member.isfile()}
            expected = {f"{root}/{name}" for name in COMMON_FILES}
            expected.update(
                f"{root}/{name}"
                for name in ("actionc", "actionc-run", "actionc-emit")
            )
            self.assertEqual(set(files), expected)
            self.assertEqual(stat.S_IMODE(files[f"{root}/actionc"].mode), 0o755)
            self.assertEqual(stat.S_IMODE(files[f"{root}/README.md"].mode), 0o644)
            build_info = package.extractfile(files[f"{root}/BUILD-INFO.txt"]).read().decode()
            mydos_notice = package.extractfile(
                files[f"{root}/licenses/MYDOS-NOTICE.md"]
            ).read().decode()
            mydos_source = package.extractfile(
                files[f"{root}/licenses/MYDOS453.ARC"]
            ).read()

        self.assertIn("channel: nightly", build_info)
        self.assertIn("commit: 0123456789abcdef", build_info)
        self.assertIn("target: x86_64-unknown-linux-musl", build_info)
        self.assertIn("rustc 1.95.0 (package test)", build_info)
        self.assertIn("David R. Eichel", mydos_notice)
        self.assertIn("source code in machine readable form", mydos_notice)
        self.assertEqual(
            hashlib.sha256(mydos_source).hexdigest(),
            "52853bdf6fa03c73cf1292c9ec6ca355f8109056d71a7531b05b51a4fdb75e87",
        )

        first_archive = archive.read_bytes()
        repeated = self.run_packager("x86_64-unknown-linux-musl")
        self.assertEqual(repeated.returncode, 0, repeated.stderr)
        self.assertEqual(archive.read_bytes(), first_archive)

    def test_windows_archive_uses_executable_suffixes(self) -> None:
        self.write_executables(".exe")
        result = self.run_packager("x86_64-pc-windows-msvc")
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.output_dir / "actionc-nightly-x86_64-pc-windows-msvc.zip"
        root = "actionc-nightly-x86_64-pc-windows-msvc"

        with zipfile.ZipFile(archive) as package:
            files = set(package.namelist())
            expected = {f"{root}/{name}" for name in COMMON_FILES}
            expected.update(
                f"{root}/{name}.exe" for name in ("actionc", "actionc-run", "actionc-emit")
            )
            self.assertEqual(files, expected)
            mode = package.getinfo(f"{root}/actionc.exe").external_attr >> 16
            self.assertEqual(stat.S_IMODE(mode), 0o755)
            readme_mode = package.getinfo(f"{root}/README.md").external_attr >> 16
            self.assertEqual(stat.S_IMODE(readme_mode), 0o644)

    def test_build_metadata_can_come_from_the_workflow_environment(self) -> None:
        self.write_executables("")
        result = self.run_packager(
            "aarch64-apple-darwin",
            metadata_from_environment=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.output_dir / "actionc-nightly-aarch64-apple-darwin.tar.gz"
        root = "actionc-nightly-aarch64-apple-darwin"

        with tarfile.open(archive, "r:gz") as package:
            build_info = package.extractfile(f"{root}/BUILD-INFO.txt").read().decode()

        self.assertIn("commit: environment-commit", build_info)
        self.assertIn("build-date: 2026-08-12T03:17:00Z", build_info)

    def test_publishable_archive_is_not_blocked_by_embedded_assets(self) -> None:
        self.write_executables("")
        result = self.run_packager("aarch64-apple-darwin")

        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.output_dir / "actionc-nightly-aarch64-apple-darwin.tar.gz"
        self.assertTrue(archive.is_file())
        with tarfile.open(archive, "r:gz") as package:
            self.assertNotIn(
                "actionc-nightly-aarch64-apple-darwin/licenses/INCOMPLETE-LICENSING.md",
                package.getnames(),
            )


if __name__ == "__main__":
    unittest.main()
