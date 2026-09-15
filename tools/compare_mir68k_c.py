#!/usr/bin/env python3
"""Build equivalent C with MC68000 GCC and compare execution in the r68k harness."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SOURCES = ROOT / "tools/mir68k-c-reference"
# Explicit source inputs and pointer slots; no inferred kernel.inc convention.
VARIANTS = {
    "insertsort": ("insertsort", "insertsort.act", ["kernel.inc"], {}),
    "matrix1": ("matrix1", "matrix1.act", ["kernel.inc"], {}),
    "matrix1-multidimensional": ("matrix1", "multidimensional.act", ["multidimensional.inc"],
                                {name: (name + "Backing", 4, 100) for name in ("matrixA", "matrixB", "matrixC")}),
    "jfdctint": ("jfdctint", "jfdctint.act", [], {}),
    "jfdctint-multidimensional": ("jfdctint", "multidimensional.act", [], {"block": ("blockBacking", 4, 64)}),
}
BENCHMARKS = tuple(VARIANTS)
ACTIONC_SWITCHES = ("no-opt", "no-codegen-opt", "no-forward-temporaries",
                   "no-select-instructions", "no-relax-branches", "no-pointer-alignment",
                   "no-control-flow", "native-promotion", "conservative-promotion",
                   "register-allocation", "no-register-allocation",
                   "guarded-memory", "no-guarded-memory")
# -mcpu selects both the instruction set and the original-68000 libgcc multilib.
# Do not add -mshort: individual C types already match the Action! declarations.
FLAGS = ["-mcpu=68000", "-std=c11", "-ffreestanding", "-fno-builtin",
         "-fwrapv", "-fno-strict-aliasing", "-fno-pic", "-fno-pie",
         "-ffunction-sections", "-fdata-sections", "-fno-common",
         "-fno-asynchronous-unwind-tables", "-Wall", "-Wextra", "-Werror"]


def capture(command):
    return subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE).stdout


def symbols(text):
    result = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) == 4:
            address, size, kind, name = fields
            size = int(size, 16)
        elif len(fields) == 3:
            address, kind, name = fields
            size = 0
        else:
            raise ValueError(f"Unrecognized nm record: {line}")
        if name in result:
            raise ValueError(f"Duplicate ELF symbol: {name}")
        result[name] = (int(address, 16), size, kind)
    return result


def build(directory, name, mode, tools, instrumented=False):
    stem = directory / f"{name}-{mode}{'-capture' if instrumented else ''}"
    flags = [*FLAGS, f"-{mode}", *(["-DACTIONC_REFERENCE_CAPTURE"] if instrumented else [])]
    source = SOURCES / f"{name}.c"
    commands = [
        [tools["gcc"], *flags, "-fstack-usage", "-c", str(source), "-o", f"{stem}.o"],
        [tools["gcc"], *flags, "-S", str(source), "-o", f"{stem}.s"],
        [tools["gcc"], *flags, "-fno-tree-loop-distribute-patterns", "-fstack-usage",
         "-c", str(SOURCES / "memory.c"), "-o", f"{stem}.memory.o"],
        [tools["gcc"], *flags, "-nostdlib", "-Wl,--gc-sections",
         f"-Wl,-T,{SOURCES / 'reference.ld'}", f"-Wl,-Map,{stem}.link-map",
         f"{stem}.o", f"{stem}.memory.o", "-lgcc", "-o", f"{stem}.elf"],
    ]
    for command in commands:
        subprocess.run(command, check=True)
    nm = capture([tools["nm"], "-n", "-S", "--defined-only", f"{stem}.elf"])
    Path(f"{stem}.nm").write_text(nm, newline="\n")
    table = symbols(nm)
    manifest = ["m68k-c-reference-v1", f"entry {table['Main'][0]:x}"]
    for section, label, permissions in [("text", "code", "rx"),
                                        ("rodata", "rodata", "r"),
                                        ("data", "data", "rw")]:
        start = table[f"__{label}_start"][0]
        size = table[f"__{label}_end"][0] - start
        if size:
            binary = Path(f"{stem}.{section}.bin")
            subprocess.run([tools["objcopy"], "--dump-section",
                            f".{section}={binary}", f"{stem}.elf"], check=True)
            if binary.stat().st_size != size:
                raise ValueError(f"Section size mismatch: {binary}")
            manifest.append(f"segment {start:x} {size} {permissions} {binary.name}")
    start = table["__bss_start"][0]
    size = table["__bss_end"][0] - start
    if size:
        manifest.append(f"zero {start:x} {size}")
    for symbol, (address, size, _) in table.items():
        if size:
            manifest.append(f"symbol {symbol} {address:x} {size}")
    for symbol, (backing, width, count) in VARIANTS[name][3].items():
        if table[symbol][1] != 4 or table[backing][1] != width * count:
            raise ValueError(f"Invalid descriptor layout: {name}/{symbol}")
        manifest.append(f"array_pointer {symbol} {backing} {width} {count}")
    Path(f"{stem}.image").write_text("\n".join(manifest) + "\n", newline="\n")
    Path(f"{stem}.dis").write_text(
        capture([tools["objdump"], "-d", "-w", f"{stem}.elf"]), newline="\n")
    return {"benchmark": name, "mode": mode, "instrumented": instrumented, "commands": commands,
            "c_source_sha256": hashlib.sha256(source.read_text().encode()).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tool-prefix", default="m68k-elf-")
    parser.add_argument("--build-dir", type=Path, default=ROOT / "build/mir68k-c-reference")
    parser.add_argument("benchmarks", nargs="*", choices=BENCHMARKS)
    promotion = parser.add_mutually_exclusive_group()
    allocation = parser.add_mutually_exclusive_group()
    guards = parser.add_mutually_exclusive_group()
    for switch in ACTIONC_SWITCHES:
        owner = promotion if switch in ("native-promotion", "conservative-promotion") else parser
        if switch in ("register-allocation", "no-register-allocation"):
            owner = allocation
        if switch in ("guarded-memory", "no-guarded-memory"):
            owner = guards
        owner.add_argument(f"--{switch}", action="store_true", help="Action! configuration only")
    args = parser.parse_args()
    actionc_switches = [f"--{switch}" for switch in ACTIONC_SWITCHES
                       if getattr(args, switch.replace("-", "_"))]
    names = args.benchmarks or BENCHMARKS
    tools = {}
    for name in ["gcc", "nm", "objcopy", "objdump"]:
        tools[name] = shutil.which(args.tool_prefix + name)
        if tools[name] is None:
            parser.error(f"Missing {args.tool_prefix + name}; see tools/mir68k-c-reference/README.md")
    directory = args.build_dir.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    # A failed rebuild must not leave a previous result looking current.
    (directory / "comparison.csv").unlink(missing_ok=True)
    metadata = {"compiler": capture([tools["gcc"], "--version"]).splitlines()[0],
                "binutils": capture([tools["objdump"], "--version"]).splitlines()[0],
                "libgcc": capture([tools["gcc"], *FLAGS, "-print-libgcc-file-name"]).strip(),
                "actionc_commit": capture(["git", "-C", str(ROOT), "rev-parse", "HEAD"]).strip(),
                "compiler_worktree_status": capture(["git", "-C", str(ROOT), "status", "--porcelain",
                                                      "--", "src", "Cargo.toml", "Cargo.lock"]),
                "actionc_switches": actionc_switches,
                "builds": []}
    inputs = [SOURCES / "memory.c", SOURCES / "reference.ld", Path(__file__).resolve()]
    for name in names:
        family, action, includes, _ = VARIANTS[name]
        fixture = ROOT / f"fixtures/runtime/tacle/{family}"
        inputs.extend([SOURCES / f"{name}.c", fixture / action, fixture / "vectors.txt",
                       *(fixture / include for include in includes)])
        if family == "jfdctint":
            inputs.extend([SOURCES / "jfdctint_impl.h", fixture / "README", fixture / "jfdctint.c"])
    inputs.extend([ROOT / "tools/vm68k-runtime-tests/tests/common/dct.rs",
                   ROOT / "tools/vm68k-runtime-tests/examples/c_reference.rs",
                   *(ROOT / "tools/vm68k-runtime-tests/examples/c_reference").glob("*.rs")])
    metadata["inputs_sha256"] = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_text().encode()).hexdigest()
                                 for path in inputs}
    for name in names:
        for mode in ["O2", "Os"]:
            metadata["builds"].append(build(directory, name, mode, tools))
            if VARIANTS[name][0] == "jfdctint":
                metadata["builds"].append(build(directory, name, mode, tools, instrumented=True))
    (directory / "toolchain.json").write_text(json.dumps(metadata, indent=2) + "\n", newline="\n")
    csv = capture(["cargo", "run", "--locked", "--manifest-path",
                   str(ROOT / "tools/vm68k-runtime-tests/Cargo.toml"),
                   "--example", "c_reference", "--", str(directory), *names, *actionc_switches])
    (directory / "comparison.csv").write_text(csv, newline="\n")
    for name in names:
        for binary in directory.glob(f"{name}-actionc-*.bin"):
            origin = binary.stem.rsplit("-", 1)[1]
            binary.with_suffix(".dis").write_text(capture([
                tools["objdump"], "-D", "-b", "binary", "-m", "m68k:68000",
                f"--adjust-vma=0x{origin}", str(binary)]), newline="\n")
    print(csv, end="")


if __name__ == "__main__":
    main()
