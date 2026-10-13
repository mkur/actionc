#!/usr/bin/env python3
"""Audit-only inlining/unrolling controls in an isolated compiler snapshot.

Default output must match each qualified XEX exactly. Only two optimized-config
flags change, through explicit environment controls in the isolated snapshot.
No compiler/source change is applied to the working checkout.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile

ROOT=Path(__file__).resolve().parents[2]
CONTROLS={"optimized":(False,False),"no_inlining":(True,False),
          "no_unrolling":(False,True),"size_control":(True,True)}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--revision",default="2e8aaa06")
    parser.add_argument("--baseline",type=Path,required=True)
    parser.add_argument("--out-dir",type=Path,required=True)
    parser.add_argument("--crlf",action="store_true")
    args=parser.parse_args()
    out=args.out_dir.resolve();out.mkdir(parents=True,exist_ok=True)
    snapshot=out/"compiler-source"
    revision=subprocess.run(["git","rev-parse",args.revision],cwd=ROOT,capture_output=True,text=True,check=True).stdout.strip()
    original=subprocess.run(["git","show",f"{revision}:src/mir6502/passes.rs"],cwd=ROOT,capture_output=True,text=True,check=True).stdout
    patched=original
    for name,env in [("enable_small_leaf_inlining","ACTIONC_SIZE_AUDIT_NO_INLINE"),
                     ("enable_small_loop_unrolling","ACTIONC_SIZE_AUDIT_NO_UNROLL")]:
        old=f"{name}: true,"
        assert patched.count(old)==1,name
        patched=patched.replace(old,f'{name}: std::env::var_os("{env}").is_none(),')
    if not snapshot.exists():
        snapshot.mkdir()
        archive=subprocess.run(["git","archive",revision],cwd=ROOT,capture_output=True,check=True).stdout
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:tar.extractall(snapshot,filter="data")
        path=snapshot/"src/mir6502/passes.rs"
        path.write_text(patched)
    assert (snapshot/"src/mir6502/passes.rs").read_text()==patched,"snapshot config does not match requested revision/controls"
    build=out/"build"
    env=os.environ.copy();env.update(CARGO_TARGET_DIR=str(build),CARGO_INCREMENTAL="0",
        CARGO_PROFILE_DEV_DEBUG="0",CARGO_PROFILE_DEV_OPT_LEVEL="1")
    env.pop("ACTIONC_SIZE_AUDIT_NO_INLINE",None);env.pop("ACTIONC_SIZE_AUDIT_NO_UNROLL",None)
    with (out/"build.log").open("w") as log:
        subprocess.run(["cargo","build","--bin","actionc"],cwd=snapshot,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
    compiler=build/"debug/actionc"
    report={"revision":args.revision,"resolved_revision":revision,"compiler_sha256":hashlib.sha256(compiler.read_bytes()).hexdigest(),
        "config_patch_sha256":hashlib.sha256((snapshot/"src/mir6502/passes.rs").read_bytes()).hexdigest(),"controls":{}}
    for program in ["TN","TNDBG"]:
        report["controls"][program]={}
        for name,(inline,unroll) in CONTROLS.items():
            control=env.copy()
            if inline:control["ACTIONC_SIZE_AUDIT_NO_INLINE"]="1"
            if unroll:control["ACTIONC_SIZE_AUDIT_NO_UNROLL"]="1"
            listing=out/f"{program}-{name}.asm";image=out/f"{program}-{name}.xex"
            with (out/f"{program}-{name}.log").open("w") as log:
                subprocess.run([str(compiler),"--mode","mir6502","--runtime","cart","--listing",str(listing),"-o",str(image),str(args.baseline/f"{program}.ACT")],cwd=ROOT,env=control,stdout=log,stderr=subprocess.STDOUT,check=True)
            blob=image.read_bytes()
            if name=="optimized":assert blob==(args.baseline/f"{program}-mir6502.xex").read_bytes(),f"{program} default mismatch"
            if args.crlf:
                other=out/f"{program}-{name}-crlf.xex"
                with (out/f"{program}-{name}-crlf.log").open("w") as log:
                    subprocess.run([str(compiler),"--mode","mir6502","--runtime","cart","-o",str(other),str(args.baseline/f"crlf/{program}.ACT")],cwd=ROOT,env=control,stdout=log,stderr=subprocess.STDOUT,check=True)
                assert other.read_bytes()==blob,f"{program}/{name} LF/CRLF mismatch"
            report["controls"][program][name]={"load_file_bytes":len(blob),"sha256":hashlib.sha256(blob).hexdigest(),"lf_crlf_identical":args.crlf}
            print(program,name,len(blob),flush=True)
    (out/"measurements.json").write_text(json.dumps(report,indent=2)+"\n")


if __name__=="__main__":main()
