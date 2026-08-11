#!/usr/bin/env python3

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
    "licenses/ALTIRRAOS-LICENSE",
    "licenses/INCOMPLETE-LICENSING.md",
    "licenses/ROM-IMAGES.md",
}


class NightlyPackageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="actionc-package-test-")
        self.root = Path(self.temporary.name)
        self.bin_dir = self.root / "bin"
        self.output_dir = self.root / "output"
        self.bin_dir.mkdir()

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def write_executables(self, suffix: str) -> None:
        for name in ("actionc", "actionc-run", "actionc-emit"):
            path = self.bin_dir / f"{name}{suffix}"
            path.write_bytes(f"fake {name}\n".encode())

    def run_packager(self, target: str, *extra: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(PACKAGER),
                "--target",
                target,
                "--bin-dir",
                str(self.bin_dir),
                "--output-dir",
                str(self.output_dir),
                "--commit",
                "0123456789abcdef",
                "--build-date",
                "2026-08-11T03:17:00Z",
                "--rustc-version",
                "rustc 1.95.0 (package test)",
                *extra,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    def test_linux_archive_has_exact_inventory_and_executable_modes(self) -> None:
        self.write_executables("")
        result = self.run_packager(
            "x86_64-unknown-linux-musl", "--allow-incomplete-license-notices"
        )
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
            build_info = package.extractfile(files[f"{root}/BUILD-INFO.txt"]).read().decode()

        self.assertIn("channel: nightly", build_info)
        self.assertIn("commit: 0123456789abcdef", build_info)
        self.assertIn("target: x86_64-unknown-linux-musl", build_info)
        self.assertIn("rustc 1.95.0 (package test)", build_info)

        first_archive = archive.read_bytes()
        repeated = self.run_packager(
            "x86_64-unknown-linux-musl", "--allow-incomplete-license-notices"
        )
        self.assertEqual(repeated.returncode, 0, repeated.stderr)
        self.assertEqual(archive.read_bytes(), first_archive)

    def test_windows_archive_uses_executable_suffixes(self) -> None:
        self.write_executables(".exe")
        result = self.run_packager(
            "x86_64-pc-windows-msvc", "--allow-incomplete-license-notices"
        )
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

    def test_publishable_archive_is_blocked_by_missing_notices(self) -> None:
        self.write_executables("")
        result = self.run_packager("aarch64-apple-darwin")

        self.assertEqual(result.returncode, 1)
        self.assertIn("ACTION-ROM-NOTICE.md", result.stderr)
        self.assertIn("MYDOS-NOTICE.md", result.stderr)
        self.assertIn("refusing to create a publishable archive", result.stderr)
        self.assertFalse(self.output_dir.exists())


if __name__ == "__main__":
    unittest.main()
