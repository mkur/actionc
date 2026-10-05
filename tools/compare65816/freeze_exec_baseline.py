#!/usr/bin/env python3
import argparse,hashlib,json,subprocess,shutil,os

# Store complete source hashes and copy external tools; never build in the live Exec checkout.
from pathlib import Path
parser=argparse.ArgumentParser(description='Freeze clean compiler HEAD and exact current Exec files into isolated worktrees')
parser.add_argument('--compiler-checkout',type=Path,required=True)
parser.add_argument('--exec-checkout',type=Path,required=True)
parser.add_argument('--base',type=Path,required=True)
args=parser.parse_args()
ROOT=args.compiler_checkout.resolve();OUT=args.base.resolve();OUT.mkdir(parents=True,exist_ok=True)
def git(root,*args):return subprocess.check_output(['git','-C',str(root),*args])
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
record={'format':'exec816-record-placement-baseline-inputs-v1'}
for directory in ('compiler','exec'):
    if (OUT/directory).exists():parser.error('Use a fresh --base; a snapshot already exists')
for name,live,dirty in [('compiler',ROOT,False),('exec',args.exec_checkout.resolve(),True)]:
    dest=OUT/name;revision=git(live,'rev-parse','HEAD').decode().strip()
    subprocess.run(['git','-C',str(live),'worktree','add','--detach',str(dest),revision],check=True)
    tracked=[Path(os.fsdecode(p)) for p in git(live,'ls-files','-z').split(b'\0') if p]
    untracked=[]
    if dirty:
        initial_status=git(live,'status','--porcelain')
        untracked=[Path(os.fsdecode(p)) for p in git(live,'ls-files','--others','--exclude-standard','-z').split(b'\0') if p and (os.fsdecode(p).startswith(('examples/','lib/','tests/','tools/','platform/','config/','toolchain/','docs/')))]
        paths=tracked+untracked
        for attempt in range(5):
            before={str(p):sha(live/p) for p in paths if (live/p).is_file()}
            for p in paths:
                src=live/p;target=dest/p
                if src.is_file():target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(src,target)
                elif target.is_file():target.unlink()
            after={str(p):sha(live/p) for p in paths if (live/p).is_file()}
            copied={str(p):sha(dest/p) for p in paths if (dest/p).is_file()}
            if before==after==copied:break
        else:raise SystemExit('Source changed during every snapshot attempt')
        if git(live,'status','--porcelain') != initial_status:
            raise SystemExit('Exec file set changed during snapshot; use a fresh destination')
        if git(live,'rev-parse','HEAD').decode().strip() != revision:
            raise SystemExit('Exec HEAD changed during snapshot; use a fresh destination')
        (OUT/'exec-working-tree.patch').write_bytes(git(live,'diff','--binary','HEAD'))
    else:paths=tracked
    files={str(p):sha(dest/p) for p in paths if (dest/p).is_file()}
    record[name]={'source':str(live),'snapshot':str(dest),'revision':revision,'files':files,'manifest_sha256':hashlib.sha256(json.dumps(files,sort_keys=True).encode()).hexdigest(),'status':git(live,'status','--porcelain').decode().splitlines() if dirty else [],'untracked_inputs':list(map(str,untracked))}
execroot=OUT/'exec';live=args.exec_checkout.resolve();external={}
for folder in ['firmware','shell-paced-bridge','altirra-irq-bridge','altirra-sio-multi']:
    src=live/'build'/folder;dst=execroot/'build'/folder;dst.mkdir(parents=True,exist_ok=True)
    paths=[p for p in src.rglob('*') if p.is_file() and (p.name in ['AltirraBridgeServer','altirraos-816.rom','ALTIRRAOS-LICENSE.txt'] or 'sdk/python' in p.as_posix() and '__pycache__' not in p.as_posix())]
    for p in paths:
        rel=p.relative_to(src);(dst/rel).parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,dst/rel);external[str(Path(folder)/rel)]=sha(dst/rel)
# SDK directories can themselves be symlinks; resolve and copy their contents.
for folder in ['shell-paced-bridge','altirra-irq-bridge','altirra-sio-multi']:
    src=(live/'build'/folder/'sdk/python').resolve();dst=execroot/'build'/folder/'sdk/python'
    shutil.copytree(src,dst,dirs_exist_ok=True,ignore=shutil.ignore_patterns('__pycache__'))
    for p in dst.rglob('*'):
        if p.is_file():external[str(p.relative_to(execroot/'build'))]=sha(p)
# Independent SDFS producer/validator needs pinned sources, headers and static libraries.
src=live/'build/altirra-irq-fix';dst=execroot/'build/altirra-irq-fix'
for relative in ['src/compat','src/h','src/ATIO/source/diskfssdx2.cpp','src/ATIO/source/diskfssdx2util.cpp',
                 'build/latency/src/ATIO/libATIO.a','build/latency/src/ATCore/libATCore.a','build/latency/src/system/libsystem.a']:
    p=src/relative;target=dst/relative
    if p.is_dir():shutil.copytree(p,target,dirs_exist_ok=True)
    else:target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(p,target)
    for q in ([target] if target.is_file() else target.rglob('*')):
        if q.is_file():external[str(q.relative_to(execroot/'build'))]=sha(q)
upstream=(live/'build/of816-upstream').resolve();dest=execroot/'build/of816-upstream'
assert not git(upstream,'status','--porcelain').strip(),'Dirty OF816 upstream'
subprocess.run(['git','-C',str(upstream),'worktree','add','--detach',str(dest),git(upstream,'rev-parse','HEAD').decode().strip()],check=True)
record['of816']={'revision':git(dest,'rev-parse','HEAD').decode().strip(),'files':{os.fsdecode(p):sha(dest/os.fsdecode(p)) for p in git(dest,'ls-files','-z').split(b'\0') if p and (dest/os.fsdecode(p)).is_file()}}
record['external_inputs']=external
pin=execroot/'toolchain/actionc.json';original=json.loads(pin.read_text());override={**original,'revision':record['compiler']['revision']};pin.write_text(json.dumps(override,indent=2)+'\n')
record['compiler_pin_override']={'original':original,'baseline':override,'path':'toolchain/actionc.json','reason':'Controlled compiler candidate; frozen source only.'}
(execroot/'build/actionc').symlink_to(OUT/'compiler',target_is_directory=True)
record['host']={'uname':subprocess.check_output(['uname','-a']).decode().strip(),'python':subprocess.check_output(['python3','--version']).decode().strip(),'rustc':subprocess.check_output(['rustc','-Vv']).decode().strip(),'cargo':subprocess.check_output(['cargo','-V']).decode().strip(),'ca65':subprocess.run(['ca65','--version'],capture_output=True,text=True).stderr.strip(),'ld65':subprocess.run(['ld65','--version'],capture_output=True,text=True).stderr.strip()}
executables={name:Path(shutil.which(name)).resolve() for name in ('python3','git','ca65','ld65','c++')}
for name in ('cargo','rustc'):executables[name]=Path(subprocess.check_output(['rustup','which',name],text=True).strip())
record['host']['executables']={name:{'path':str(path),'sha256':sha(path)} for name,path in executables.items()}
record['host']['cxx_version']=subprocess.check_output(['c++','--version'],text=True).strip()
(OUT/'inputs.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({name:{k:v for k,v in record[name].items() if k not in ['files','status']} for name in ['compiler','exec']},indent=2))
print('Frozen external input files:',len(external))
