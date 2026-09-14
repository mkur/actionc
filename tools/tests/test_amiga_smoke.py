import importlib.util
import json
import re
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location("amiga_smoke", Path(__file__).resolve().parents[1] / "amiga_smoke.py")
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


class AmigaSmokeTests(unittest.TestCase):
    def test_script_captures_each_status_immediately_and_attempts_recovery(self):
        text = SMOKE.script("a" * 32).decode("ascii")
        self.assertNotIn("\r", text)
        for line, following in zip(text.splitlines(), text.splitlines()[1:]):
            if ".amiga" in line:
                self.assertRegex(following, r"^Set [A-Za-z][A-Za-z0-9]* \$RC$")
        self.assertIn("FailAt 21", text)
        self.assertLess(text.index("RC division-zero.redirect"), text.index("RC hello.recovery"))

    def test_saved_status_survives_classic_shell_variable_expansion(self):
        # RKRM AmigaDOS 15.1.5: an unbraced variable ends at the first
        # non-alphanumeric character; unknown variables remain literal.
        # Apply that grammar independently to the generated command lines.
        def expand(line, variables):
            return re.sub(
                r"\$([A-Za-z0-9]+)",
                lambda match: variables.get(match[1].lower(), match[0]),
                line,
            )

        self.assertEqual(expand("$actionc_rc", {"actionc_rc": "20"}), "$actionc_rc")
        lines = SMOKE.script("a" * 32).decode("ascii").splitlines()
        commands = 0
        for index, line in enumerate(lines):
            if not re.match(r"^[a-z-]+\.amiga(?:\s|$)", line):
                continue
            commands += 1
            capture = re.fullmatch(r"Set (\S+) (.+)", lines[index + 1])
            self.assertIsNotNone(capture)
            for rc in (0, 5, 20):
                with self.subTest(command=line, rc=rc):
                    variables = {"rc": str(rc)}
                    variables[capture[1].lower()] = expand(capture[2], variables)
                    variables["rc"] = "0"  # Set/Echo must not lose the saved result.
                    logged = expand(lines[index + 2], variables)
                    checked = expand(lines[index + 3], variables)
                    self.assertRegex(logged, rf'^Echo "RC [a-z.-]+ {rc}" >>RAM:actionc-status.txt$')
                    self.assertRegex(checked, rf"^If NOT {rc} EQ (0|20) VAL$")
        self.assertEqual(commands, 9)

    def test_verification_requires_current_marker_exact_output_statuses_and_os_versions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bundle, collected = root / "bundle", root / "collected"
            (bundle / "expected").mkdir(parents=True)
            collected.mkdir()
            run_id = "a" * 32
            files = {}
            for name in SMOKE.SAMPLES:
                (bundle / f"{name}.amiga").write_bytes(b"independent fixture")
                (bundle / "expected" / f"{name}.txt").write_bytes(SMOKE.expected(name))
                (collected / f"actionc-{name}.txt").write_bytes(SMOKE.expected(name))
            (bundle / "smoke").write_bytes(SMOKE.script(run_id))
            for path in bundle.rglob("*"):
                if path.is_file():
                    files[path.relative_to(bundle).as_posix()] = SMOKE.digest(path)
            (bundle / "manifest.json").write_text(json.dumps({"version": 1, "run_id": run_id, "files": files}))
            good_status = "\n".join([f"BUILD {run_id}", *(f"RC {label} {rc}" for label, _, _, rc in SMOKE.runs()), "SMOKE PASS"]) + "\n"
            status = collected / "actionc-status.txt"
            status.write_bytes(good_status.encode())
            for name in ("exec", "dos"):
                (collected / f"actionc-{name}-version.txt").write_bytes(f"{name}.library 40.3\n".encode())
            SMOKE.verify(bundle, collected)
            for invalid in [good_status.replace("SMOKE PASS\n", ""), good_status.replace(run_id, "b" * 32), good_status.replace("RC division-zero.redirect 20", "RC division-zero.redirect 0"), good_status.replace("\n", "\r\n")]:
                status.write_bytes(invalid.encode())
                with self.assertRaises(ValueError):
                    SMOKE.verify(bundle, collected)
            status.write_bytes(good_status.replace("RC hello.console 0", "RC hello.console $actionc_rc").encode())
            with self.assertRaisesRegex(ValueError, "unexpanded Shell variable"):
                SMOKE.verify(bundle, collected)
            status.write_bytes(good_status.encode())
            report = collected / "actionc-insertsort.txt"
            original = report.read_bytes()
            report.write_bytes(original.replace(b"2 3", b"3 2"))
            with self.assertRaisesRegex(ValueError, "insertsort"):
                SMOKE.verify(bundle, collected)
            report.write_bytes(original)
            (collected / "actionc-exec-version.txt").write_bytes(b"exec.library 37.175\n")
            with self.assertRaisesRegex(ValueError, "version 40"):
                SMOKE.verify(bundle, collected)


if __name__ == "__main__":
    unittest.main()
