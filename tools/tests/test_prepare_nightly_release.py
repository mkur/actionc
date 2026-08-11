#!/usr/bin/env python3

import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
PACKAGER = REPO_ROOT / "tools/package-nightly.py"
PREPARER = REPO_ROOT / "tools/prepare-nightly-release.py"
ACTION_SOURCE = REPO_ROOT / "roms/source/action-3.6-source-0b8bcedb.tar.gz"
COMMIT = "0123456789abcdef0123456789abcdef01234567"
TARGETS = (
    "x86_64-unknown-linux-musl",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
)
PACKAGE_NAMES = {
    "actionc-nightly-x86_64-unknown-linux-musl.tar.gz",
    "actionc-nightly-x86_64-pc-windows-msvc.zip",
    "actionc-nightly-aarch64-apple-darwin.tar.gz",
    "actionc-nightly-x86_64-apple-darwin.tar.gz",
}
ACTION_SOURCE_NAME = "action-3.6-source-0b8bcedb.tar.gz"


class NightlyReleasePreparationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(
            prefix="actionc-release-prepare-test-"
        )
        self.root = Path(self.temporary.name)
        self.input_dir = self.root / "input"
        self.output_dir = self.root / "output"
        self.input_dir.mkdir()
        self.make_packages(COMMIT)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    def make_packages(self, commit: str) -> None:
        for target in TARGETS:
            suffix = ".exe" if target.endswith("windows-msvc") else ""
            bin_dir = self.root / f"bin-{target}"
            bin_dir.mkdir(exist_ok=True)
            for executable in ("actionc", "actionc-run", "actionc-emit"):
                (bin_dir / f"{executable}{suffix}").write_bytes(
                    f"fake {target} {executable}\n".encode()
                )
            result = subprocess.run(
                [
                    sys.executable,
                    str(PACKAGER),
                    "--target",
                    target,
                    "--bin-dir",
                    str(bin_dir),
                    "--output-dir",
                    str(self.input_dir),
                    "--commit",
                    commit,
                    "--build-date",
                    "2026-08-11T03:17:00Z",
                    "--rustc-version",
                    "rustc 1.95.0 (release preparation test)",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def run_preparer(
        self, *extra: str, commit: str = COMMIT
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(PREPARER),
                "--input-dir",
                str(self.input_dir),
                "--output-dir",
                str(self.output_dir),
                "--action-source",
                str(ACTION_SOURCE),
                "--commit",
                commit,
                *extra,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

    def test_prepares_exact_release_inventory_and_verified_checksums(self) -> None:
        result = self.run_preparer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(Path(result.stdout.strip()), self.output_dir.resolve())

        expected = PACKAGE_NAMES | {ACTION_SOURCE_NAME, "SHA256SUMS"}
        self.assertEqual({path.name for path in self.output_dir.iterdir()}, expected)
        self.assertEqual(
            (self.output_dir / ACTION_SOURCE_NAME).read_bytes(),
            ACTION_SOURCE.read_bytes(),
        )

        checksum_lines = (self.output_dir / "SHA256SUMS").read_text().splitlines()
        self.assertEqual(len(checksum_lines), 5)
        for line in checksum_lines:
            digest, name = line.split("  ", 1)
            self.assertEqual(
                digest,
                hashlib.sha256((self.output_dir / name).read_bytes()).hexdigest(),
            )

    def test_rejects_unexpected_matrix_artifact(self) -> None:
        (self.input_dir / "unexpected.txt").write_text("unexpected\n")
        result = self.run_preparer()

        self.assertEqual(result.returncode, 1)
        self.assertIn("nightly input inventory mismatch", result.stderr)
        self.assertFalse(self.output_dir.exists())

    def test_rejects_archive_from_another_commit(self) -> None:
        result = self.run_preparer(
            commit="ffffffffffffffffffffffffffffffffffffffff"
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("inconsistent BUILD-INFO.txt", result.stderr)
        self.assertFalse(self.output_dir.exists())


if __name__ == "__main__":
    unittest.main()
