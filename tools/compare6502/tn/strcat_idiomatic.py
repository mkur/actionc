#!/usr/bin/env python3
"""Assemble handwritten Strcat controls and bind them to qualified TN images."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from strcat_breakdown import parse_action, xex_memory

ROOT = Path(__file__).resolve().parents[3]
DOC = ROOT / "docs/benchmarks/6502-strcat-idiomatic"
BUILD = ROOT / "build/strcat-idiomatic"
MEASURED = ROOT / "build/argument-x-bitwise/measured"
QUALIFIED = ROOT / "docs/benchmarks/6502-argument-x-bitwise/results.json"
VARIANTS = {"compiler": 113, "carry-only": 107, "direct-destination": 99,
            "same-abi": 97, "private-pointers": 52}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def table(text):
    return {key: int(value, 16) for key, value in
            (line.split("\t") for line in text.splitlines())}


def crlf(text):
    return text.replace("\r\n", "\n").replace("\n", "\r\n")


def sources(compiler):
    same = (DOC / "same-abi.asm").read_text().replace("\r\n", "\n")
    private = (DOC / "private-pointers.asm").read_text().replace("\r\n", "\n")
    # Diagnostic assembly splices, not compiler rewrites. Keep every instruction
    # outside the explicitly bounded destination calculation unchanged.
    first = compiler.index("        LDY #$00")
    last = compiler.index("        LDA.Z $AC", first)
    carry = compiler[:first] + """        LDY #$00
        LDA.Z $AE
        SEC
        ADC ($AE),Y
        STA.Z $AC
        LDA.Z $AF
        ADC #$00
        STA.Z $AD
""" + compiler[last:]
    marker = "        lda ($AC),y             ; Y is still zero."
    assert same.count(marker) == 1
    direct = same.replace(marker, "        ldy #0\n" + marker)
    return {"compiler": compiler, "carry-only": carry,
            "direct-destination": direct, "same-abi": same, "private-pointers": private}


def assemble(text, stem, origin, params, move_page, windows):
    prefix = (f"        org ${origin:04X}\n"
              f"param_Strcat_s = ${params['Strcat.s']:04X}\n"
              f"param_Strcat_t = ${params['Strcat.t']:04X}\n"
              f"proc_MovePage = ${move_page:04X}\n")
    path = BUILD / f"{stem}-{'crlf' if windows else 'lf'}.asm"
    output = path.with_suffix(".xex")
    source = prefix + text
    path.write_bytes((crlf(source) if windows else source).encode())
    result = subprocess.run([os.environ.get("ACTIONC_MADS", "mads"), str(path),
                             f"-o:{output}", "-s"], capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr
    image = output.read_bytes()
    memory = xex_memory(image)
    assert sorted(memory) == list(range(origin, origin + len(memory)))
    return image, bytes(memory[a] for a in sorted(memory)), output


def measure(check, assemble_only):
    BUILD.mkdir(parents=True, exist_ok=True)
    qualified = json.loads(QUALIFIED.read_text())
    for path in ["samples/tn/modern/TN.ACT", "samples/tn/modern/TNDBG.ACT",
                 "samples/tn/modern/LIB.ACT"]:
        assert sha((ROOT / path).read_text().replace("\r\n", "\n").encode()) == qualified["inputs_sha256"][path]
    canonical = (MEASURED / "TN-ActionCart-true.asm").read_text().replace("\r\n", "\n")
    start = canonical.index("; ===== PROC Strcat ")
    end = canonical.index("; ===== END PROC Strcat =====", start)
    compiler = canonical[start:end] + "; ===== END PROC Strcat =====\n"
    current_file = DOC / "compiler.asm"
    if check:
        assert current_file.read_text().replace("\r\n", "\n") == compiler
    else:
        current_file.write_text(compiler)
    controls = sources(compiler)
    report = {"compiler_revision": "d0c947cd2a668a6f3a17204d2a93c6f840501482",
              "qualified_measurement": QUALIFIED.relative_to(ROOT).as_posix(),
              "vm_revision": qualified["vm_revision"],
              "source_sha256": {p: qualified["inputs_sha256"][p] for p in
                                ["samples/tn/modern/TN.ACT", "samples/tn/modern/TNDBG.ACT",
                                 "samples/tn/modern/LIB.ACT"]},
              "body_bytes": VARIANTS, "controls": [], "assemblies": []}
    report["audit_source_sha256"] = {
        p.relative_to(ROOT).as_posix(): sha(p.read_text().replace("\r\n", "\n").encode())
        for p in [current_file, DOC / "same-abi.asm", DOC / "private-pointers.asm",
                  ROOT / "tools/compare6502/tn/strcat_idiomatic.py",
                  ROOT / "tools/vm-runtime-tests/tests/strcat_idiomatic.rs"]}
    manifest = []
    for name in ["TN", "TNDBG"]:
        for runtime in ["ActionCart", "Standalone"]:
            stem = f"{name}-{runtime}-true"
            pair = next(p for p in qualified["pairs"] if p["name"] == f"{name}-{runtime}")
            image_path = MEASURED / f"{stem}.xex"
            image = image_path.read_bytes()
            assert sha(image) == pair["image_sha256"][1]
            listing = (MEASURED / f"{stem}.asm").read_text()
            parsed = parse_action(listing)
            assert parsed == parse_action(crlf(listing))
            entry, end, rows = parsed
            raw = bytes(b for row in rows for b in row["bytes"])
            memory = xex_memory(image)
            assert raw == bytes(memory[a] for a in range(entry, end))
            assert len(raw) == VARIANTS["compiler"]
            params_text = (MEASURED / f"{stem}.params").read_text()
            entry_text = (MEASURED / f"{stem}.entries").read_text()
            params, entries = table(params_text), table(entry_text)
            assert params == table(crlf(params_text))
            assert entries == table(crlf(entry_text))
            helper = bytes(memory[a] for a in range(entries["MovePage"], entries["MovePage"] + 19))
            assert helper == bytes.fromhex("85 A0 86 A1 84 A2 A4 A4 88 B1 A2 91 A0 88 C0 FF D0 F7 60")
            report["controls"].append({"name": stem, "image_sha256": sha(image),
                                       "strcat_start": entry, "strcat_bytes": len(raw),
                                       "move_page": entries["MovePage"],
                                       "s_home": params["Strcat.s"], "t_home": params["Strcat.t"]})
            for origin in [0x0800, 0x09C7]:
                for variant, text in controls.items():
                    output_stem = f"{name}-{runtime}-{origin:04X}-{variant}"
                    lf, body, output = assemble(text, output_stem, origin, params, entries["MovePage"], False)
                    windows, windows_body, _ = assemble(text, output_stem, origin, params, entries["MovePage"], True)
                    assert lf == windows and body == windows_body
                    assert len(body) == VARIANTS[variant], (variant, len(body))
                    if variant == "compiler":
                        assert body == raw  # The only local jump is relative.
                    report["assemblies"].append({"name": output_stem, "bytes": len(body),
                                                  "image_sha256": sha(lf)})
                    manifest.append("\t".join([name, runtime, variant, f"{origin:04X}",
                                               f"{entries['MovePage']:04X}", f"{params['Strcat.s']:04X}",
                                               f"{params['Strcat.t']:04X}", str(len(body)),
                                               output.relative_to(ROOT).as_posix(), image_path.relative_to(ROOT).as_posix(),
                                               f"{entry:04X}", lf.hex()]))
    manifest_text = "\n".join(manifest) + "\n"
    (BUILD / "manifest.tsv").write_text(manifest_text)
    if assemble_only:
        print(f"Assembled {len(manifest) * 2} LF/CRLF controls; wrote {BUILD / 'manifest.tsv'}")
        return
    vm_text = (BUILD / "vm-results.tsv").read_text()
    assert (BUILD / "vm-manifest.tsv").read_text() == manifest_text, "rerun VM check for current fragments"
    samples, counts = [], {}
    for line in vm_text.splitlines():
        fields = line.split("\t")
        if fields[0] == "count":
            counts[fields[1]] = int(fields[2])
        else:
            name, runtime, variant, origin, scenario, body, helper, wrapper = fields
            samples.append(dict(name=name, runtime=runtime, variant=variant, origin=int(origin, 16),
                                scenario=scenario, body_cycles=int(body), helper_cycles=int(helper),
                                wrapper_cycles=int(wrapper), total_cycles=int(body)+int(helper)+int(wrapper)))
    report.update(vm_samples=samples, vm_checks=counts, lf_crlf_assemblies=len(manifest)*2)
    assert counts == dict(normal_calls=9600, parameter_alias_calls=64,
                          private_counterexamples=16, scratch_alias_calls=96)
    assert len(samples) == len(manifest)*3
    for sample in samples:
        assert sample["wrapper_cycles"] == 17
        assert sample["helper_cycles"] == {"3+8": 163, "low-carry": 37, "zero-means-256": 4627}[sample["scenario"]]
    target = DOC / "results.json"
    if check:
        assert json.loads(target.read_text()) == report, "Strcat idiomatic report differs"
    else:
        target.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Verified {len(report['controls'])} qualified images, {len(manifest)*2} assemblies and {counts}")
    for variant, size in VARIANTS.items():
        sample = next(s for s in samples if s["variant"] == variant and s["scenario"] == "3+8")
        print(f"{variant}: {size} bytes, {sample['total_cycles']} cycles (3+8)")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--assemble-only", action="store_true")
    args = parser.parse_args()
    measure(args.check, args.assemble_only)
