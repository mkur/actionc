#!/usr/bin/env python3
"""Byte-exact size attribution for qualified 6502 images and listings.

Code buckets are disjoint physical encodings, not semantic overhead or
predictions of removable bytes. Source-confirmed popup payloads are data even
when the listing prints them as instructions. Motifs overlap buckets and carry
no alias proof.
Only the analysis and reports change; no compiler rewrite is performed.
"""
import argparse
from collections import Counter,defaultdict
import csv
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import sys
import tempfile

sys.dont_write_bytecode=True
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location("tn_census",Path(__file__).with_name("audit-current-output.py"))
audit=importlib.util.module_from_spec(spec);spec.loader.exec_module(audit)
BRANCHES={0x10,0x30,0x50,0x70,0x90,0xB0,0xD0,0xF0}
INDIRECT={0x01,0x21,0x41,0x61,0x81,0xA1,0xC1,0xE1,0x11,0x31,0x51,0x71,0x91,0xB1,0xD1,0xF1}
INDEXED={0x15,0x35,0x55,0x75,0x95,0xB5,0xD5,0xF5,0x19,0x39,0x59,0x79,0x99,0xB9,0xD9,0xF9,
         0x1D,0x3D,0x5D,0x7D,0x9D,0xBD,0xDD,0xFD,0xB4,0x94,0xB6,0x96,0xBC,0xBE,0x16,0x36,0x56,0x76,0xD6,0xF6,0x1E,0x3E,0x5E,0x7E,0xDE,0xFE}
BUCKETS=("call","tail_call","branch_jump","return","pointer_area","abi_area","named_spill","parameter_home","local_home","global_or_static","other_zp","other_absolute","indexed_memory","indirect_memory","index_register","source_alias","other")
DECL=re.compile(r"^(?:(?:PUBLIC|INLINE)\s+)*(?:(?:BYTE|CARD|INT|CHAR)\s+)?(?:FUNC|PROC)\s+(\w+)\s*(?:=\*)?\s*\(",re.I)


def sha(data):return hashlib.sha256(data).hexdigest()


def inventory(text):
    """Keep consecutive alias labels, but start a new object after its bytes.

    Classic prints several parameter/local objects under one storage banner.
    A previous label must not extend into the next object's data rows.
    """
    rows,ranges,_=audit.parse_listing(text)
    aliases={};active=[];had_data=False;label_addresses={};pending=[]
    for line in text.splitlines():
        if line.startswith("; ====="):
            active=[];had_data=False;pending=[]
        if match:=audit.FIXED.fullmatch(line):aliases[match[1]]={int(match[2],16)}
        if match:=audit.LABEL.fullmatch(line):
            if had_data:active=[];had_data=False
            active.append(match[1]);pending.append(match[1])
        if match:=audit.ROW.match(line):
            addr=int(match[2],16);data=bytes.fromhex(match[3]);form=match[1].strip()
            for name in pending:label_addresses[name]=addr
            pending=[]
            if form.startswith((".BYTE",".WORD","DTA ")):
                for name in active:aliases.setdefault(name,set()).update(range(addr,addr+len(data)))
                had_data=True
            else:active=[];had_data=False
    return rows,ranges,aliases,label_addresses


def source_inventory(directory,program):
    result={}
    for path in [*(directory/f"{name}.ACT" for name in ["LIB","DIR","MYDOS","LOCATION","PANELDIR"]),directory/f"{program}.ACT"]:
        text=path.read_text().replace("\r\n","\n");lines=text.splitlines()
        declarations=[(i,DECL.match(line)) for i,line in enumerate(lines) if DECL.match(line)]
        menu_section=False;previous=0;menus=[]
        for ordinal,(i,match) in enumerate(declarations):
            if any("Data for popup menus" in line for line in lines[previous:i]):menu_section=True
            end=declarations[ordinal+1][0] if ordinal+1<len(declarations) else len(lines)
            body=[line.strip() for line in lines[i+1:end] if line.strip() and not line.lstrip().startswith(";")]
            raw_menu=menu_section and bool(body) and body[0].startswith("[")
            if raw_menu:
                assert '"' in "\n".join(body) and body[-1].endswith("]"),match[1]
                menus.append((i,match[1]))
            else:menu_section=False
            previous=i
            # Pure machine bodies have no generated Action statements after ASM.
            pure=False
            if body and body[0] in {"ASM","ASM OPAQUE"} and "ENDASM" in body:
                tail=body[body.index("ENDASM")+1:]
                pure=not tail or tail[0]=="MODULE" or tail[0].startswith(("BYTE ","CARD ","CONST ","TYPE "))
            result[match[1].lower()]={"source":str(path.relative_to(directory)),"line":i+1,"pure_asm":pure,"raw_menu_data":raw_menu}
        # This is a corpus-specific diagnostic interpretation, not a compiler
        # rule for raw PROC bodies. Confirm every non-declaration use is @name.
        for declaration,name in menus:
            for i,line in enumerate(lines):
                if i==declaration:continue
                host=re.sub(r'"[^"\n]*"',"",line.split(";",1)[0])
                for use in re.finditer(r"\b"+re.escape(name)+r"\b",host,re.I):
                    assert use.start()>0 and host[use.start()-1]=="@",(path,i+1,name)
    return result


def home_sets(aliases):
    return {kind:set().union(*(v for k,v in aliases.items() if k.startswith(kind+"_"))) for kind in ["spill","param","local","global","static_string"]}


def bucket(row,sets,entries):
    d=row["data"];op=d[0]
    if op==0x20:return "call"
    if op==0x4C and int.from_bytes(d[1:3],"little") in entries:return "tail_call"
    if op in BRANCHES or op in {0x4C,0x6C}:return "branch_jump"
    if op in {0x60,0x40}:return "return"
    if op in INDIRECT:return "indirect_memory"
    if op in INDEXED:return "indexed_memory"
    address=audit.direct_operand(d)
    if address is not None:
        if 0xAA<=address<=0xAF:return "pointer_area"
        if 0xA0<=address<=0xA5:return "abi_area"
        kinds=[kind for kind in ["spill","param","local","global","static_string"] if address in sets[kind]]
        if len(kinds)>1:return "source_alias"
        if kinds:return {"spill":"named_spill","param":"parameter_home","local":"local_home","global":"global_or_static","static_string":"global_or_static"}[kinds[0]]
        return "other_zp" if address<256 else "other_absolute"
    if op in {0xA0,0xA2,0xA8,0xAA,0x98,0x8A,0xC8,0xE8,0x88,0xCA}:return "index_register"
    return "other"


def motifs(rows,ranges,labels):
    found=[];grouped=defaultdict(list)
    for row in rows:grouped[row["routine"]].append(row)
    for name,ops in grouped.items():
        if name is None:continue
        targets=set(labels.values())
        block=0;blocks=[]
        for row in ops:
            if row["address"] in targets:block+=1
            blocks.append(block)
            if row["directive"] or row["data"][0] in BRANCHES|{0x20,0x4C,0x6C,0x60,0x40}:block+=1
        previous={}
        for i in range(len(ops)-3):
            window=ops[i:i+4]
            if any(r["directive"] for r in window):continue
            a,b,c,d=(r["data"] for r in window)
            if a[0] not in {0xA5,0xAD,0xA9} or c[0] not in {0xA5,0xAD,0xA9} or b[0]!=0x85 or d[0]!=0x85:continue
            if b[1] not in {0xAA,0xAC,0xAE} or d[1]!=b[1]+1:continue
            if any(x["address"]+len(x["data"])!=y["address"] for x,y in zip(window,window[1:])):continue
            source=(a.hex(),c.hex());key=(blocks[i],b[1],source)
            prev=previous.get(key);barriers=[]
            if prev is not None:
                for row in ops[prev+4:i]:
                    op=row["data"][0];address=audit.direct_operand(row["data"])
                    if op in {0x81,0x91}:barriers.append("indirect_write")
                    if op in audit.WRITES|audit.RMW and address in {b[1],d[1]}:barriers.append("pair_write")
                    source_cells={int.from_bytes(lane[1:],"little") for lane in [a,c] if lane[0]!=0xA9}
                    if op in audit.WRITES|audit.RMW and address in source_cells:barriers.append("base_cell_write")
                if not any(x in barriers for x in ["pair_write","base_cell_write"]):
                    found.append(dict(kind="repeated_pair_setup",routine=name,address=window[0]["address"],previous_address=ops[prev]["address"],bytes=sum(len(r["data"]) for r in window),pair=b[1],conditions=",".join(sorted(set(barriers))) or "no_detected_write_barrier",forms=" | ".join(r["form"] for r in window)))
            previous[key]=i
            found.append(dict(kind="pair_setup",routine=name,address=window[0]["address"],previous_address="",bytes=sum(len(r["data"]) for r in window),pair=b[1],conditions="unproved",forms=" | ".join(r["form"] for r in window)))
        # Numeric Boolean materialization: both constants 0/1 before a join.
        for i in range(len(ops)-3):
            w=ops[i:i+4]
            if any(r["directive"] for r in w):continue
            if w[0]["data"] in [b"\xA9\x00",b"\xA9\x01"] and w[1]["data"][0]==0x4C and w[2]["data"] in [b"\xA9\x00",b"\xA9\x01"] and w[0]["data"]!=w[2]["data"] and int.from_bytes(w[1]["data"][1:],"little")==w[3]["address"]:
                found.append(dict(kind="boolean_materialization",routine=name,address=w[0]["address"],previous_address="",bytes=7,pair="",conditions="consumer_may_require_numeric_value",forms=" | ".join(r["form"] for r in w[:3])))
        for i in range(len(ops)-5):
            w=ops[i:i+6]
            if any(r["directive"] for r in w):continue
            d=[r["data"] for r in w]
            if d[0] in [b"\xA9\x00",b"\xA9\x01"] and d[1][0] in {0x85,0x8D} and d[2][0]==0x4C and d[3] in [b"\xA9\x00",b"\xA9\x01"] and d[0]!=d[3] and d[1]==d[4] and int.from_bytes(d[2][1:],"little")==w[5]["address"]:
                found.append(dict(kind="stored_boolean_materialization",routine=name,address=w[0]["address"],previous_address="",bytes=sum(len(x) for x in d[:5]),pair="",conditions="consumer_may_require_numeric_value_and_home",forms=" | ".join(r["form"] for r in w[:5])))
        # Identical literal BYTE exits within one routine. Raw saving of sharing
        # a later five-byte exit through a three-byte JMP is two bytes per copy;
        # final layout/ABI/timing qualification remains a compiler obligation.
        exits=defaultdict(list)
        for i in range(len(ops)-2):
            w=ops[i:i+3]
            if any(r["directive"] for r in w):continue
            if w[0]["data"][0]==0xA9 and w[1]["data"]==b"\x85\xA0" and w[2]["data"]==b"\x60":
                exits[w[0]["data"][1]].append(w)
        for value,windows in exits.items():
            for w in windows[:-1]:
                found.append(dict(kind="duplicate_literal_byte_exit",routine=name,address=w[0]["address"],previous_address=windows[-1][0]["address"],bytes=5,pair="",conditions=f"return_value_{value};two_byte_raw_saving",forms=" | ".join(r["form"] for r in w)))
        # Captured word stores shuttle low A through X and high A through stack.
        for i in range(len(ops)-8):
            w=ops[i:i+9]
            if any(r["directive"] for r in w):continue
            d=[r["data"] for r in w]
            if d[0]==b"\xAA" and d[1][0] in {0xAD,0xA5,0xA9} and d[2]==b"\x48" and d[3]==b"\x8A" and d[4][0]==0xA0 and d[5][0]==0x91 and d[6]==b"\x68" and d[7]==b"\xC8" and d[8]==d[5]:
                found.append(dict(kind="word_store_stack_shuttle",routine=name,address=w[0]["address"],previous_address="",bytes=4,pair=d[5][1],conditions="preserves_full_capture_and_store_byte_order",forms=" | ".join(r["form"] for r in w)))
    return found


def boundaries(rows,ranges,aliases,labels):
    grouped=defaultdict(list)
    for row in rows:grouped[row["routine"]].append(row)
    entries={r["entry"]:name for name,r in ranges.items()}
    counts=Counter();incoming=Counter();by_routine={}
    for name,ops in grouped.items():
        if name is None:continue
        own=set().union(*(v for k,v in aliases.items() if k.lower().startswith(f"param_{name.lower()}_")))
        capture=0;sargs=0
        entry_ops=[r for r in ops if r["address"]>=ranges[name]["entry"]]
        for i,row in enumerate(entry_ops):
            data=row["data"]
            if row["directive"]:break
            address=audit.direct_operand(data)
            if data[0] in {0x85,0x86,0x84,0x8D,0x8E,0x8C} and address in own:capture+=len(data)
            elif data[0]==0xA5 and 0xA3<=data[1]<=0xAF:capture+=len(data)
            elif i==0 and data==b"\x20\xF5\xA0":sargs=6;break
            else:break
        suffix=0
        targets=set(labels.values())
        for i,row in enumerate(ops):
            d=row["data"]
            if row["directive"] or d[0] not in {0x20,0x4C}:continue
            target=int.from_bytes(d[1:3],"little")
            if d[0]==0x4C and target not in entries:continue
            if target in entries:incoming[entries[target]]+=1
            j=i-1;size=0;boundary=row["address"]
            while j>=0:
                prev=ops[j];op=prev["data"][0]
                if prev["directive"] or prev["address"]+len(prev["data"])!=boundary:break
                if op not in {0xA9,0xA5,0xAD,0xA2,0xA6,0xAE,0xA0,0xA4,0xAC,0xAA,0xA8,0x8A,0x98}:break
                size+=len(prev["data"]);boundary=prev["address"]
                if prev["address"] in targets:break
                j-=1
            suffix+=size
        by_routine[name]={"entry_capture_bytes":capture,"sargs_entry_bytes":sargs,"call_load_transfer_suffix_bytes":suffix}
        counts.update(by_routine[name])
    return dict(counts),by_routine,dict(incoming)


def analyze(listing,blob,sources):
    audit.census(listing,blob) # Every annotated byte, plus symbolic RUNAD, reconciles.
    rows,ranges,aliases,labels=inventory(listing);sets=home_sets(aliases)
    raw_menus={name for name in ranges if sources.get(name.lower(),{}).get("raw_menu_data",False)}
    form_code=sum(len(r["data"]) for r in rows if not r["directive"])
    form_data=sum(len(r["data"]) for r in rows if r["directive"])
    # Preserve printed forms separately; data that decodes as an opcode must
    # not contribute fake calls, branches, ABI traffic, or compiler motifs.
    rows=[dict(r,directive=r["directive"] or r["routine"] in raw_menus) for r in rows]
    entries={r["entry"] for name,r in ranges.items() if name not in raw_menus}
    by_routine={};totals=Counter();data_roles=Counter();modules=Counter()
    directive_addresses={r["address"]+i for r in rows if r["directive"] for i in range(len(r["data"]))}
    menu_addresses={r["address"]+i for r in rows if r["routine"] in raw_menus for i in range(len(r["data"]))}
    for address in directive_addresses:
        if address in menu_addresses:data_roles["raw_menu_payload"]+=1;continue
        kinds=tuple(k for k in ["spill","param","local","global","static_string"] if address in sets[k])
        data_roles["+".join(kinds) or "inline_or_unlabelled"]+=1
    for name,extent in ranges.items():
        owned=[r for r in rows if r["routine"]==name]
        source=sources.get(name.lower(),{"source":"runtime" if name.startswith("ACTION.RUNTIME") else "data_or_generated","line":0,"pure_asm":False,"raw_menu_data":False})
        by_routine[name]={"start":extent["entry"],"code_bytes":0,"data_bytes":0,"entry_capture_bytes":0,"sargs_entry_bytes":0,"call_load_transfer_suffix_bytes":0,"buckets":Counter(),**source}
        for row in owned:
            n=len(row["data"])
            if row["directive"]:by_routine[name]["data_bytes"]+=n
            else:
                kind=bucket(row,sets,entries);by_routine[name]["buckets"][kind]+=n
                by_routine[name]["code_bytes"]+=n;totals[kind]+=n
                modules[source["source"]]+=n
    # Some listings can contain instruction rows outside a PROC.
    outside=sum(len(r["data"]) for r in rows if r["routine"] is None and not r["directive"])
    assert outside==0,outside
    code=sum(totals.values());data=len(directive_addresses)
    assert code+data+2==len(audit.xex_memory(blob))
    assert len(blob)==code+data+12
    sites=motifs(rows,ranges,labels)
    boundary_totals,boundary_routines,incoming=boundaries(rows,ranges,aliases,labels)
    for name,details in boundary_routines.items():by_routine[name].update(details)
    return {"load_file_bytes":len(blob),"code_bytes":code,"data_bytes":data,
        "instruction_form_bytes":form_code,"directive_form_bytes":form_data,
        "xex_headers":10,"runad_bytes":2,"code_buckets":dict(totals),"data_roles":dict(data_roles),
        "boundary_patterns":boundary_totals,"static_incoming_calls":incoming,"source_units_code_bytes":dict(modules),"pure_asm_code_bytes":sum(r["code_bytes"] for r in by_routine.values() if r["pure_asm"]),
        "image_sha256":sha(blob),"listing_sha256_lf":sha(listing.replace("\r\n","\n").encode()),
        "routines":by_routine,"motifs":sites}


def check():
    text="""; ===== PROC Demo $3000..$3014 entry $3006 =====
; ===== DATA Demo storage $3000 =====
param_Demo_p:
        .BYTE $00,$00 ; $3000: 00 00
local_Demo_x:
        .BYTE $00,$00,$00,$00 ; $3002: 00 00 00 00
proc_Demo:
        LDA.A param_Demo_p ; $3006: AD 00 30
        STA.Z $AC ; $3009: 85 AC
        LDA.A param_Demo_p+1 ; $300B: AD 01 30
        STA.Z $AD ; $300E: 85 AD
        LDA.A local_Demo_x ; $3010: AD 02 30
        RTS ; $3013: 60
; ===== END PROC Demo =====
        ORG $02E2
        DTA A(proc_Demo)
"""
    blob=bytes.fromhex("FF FF 00 30 13 30 00 00 00 00 00 00 AD 00 30 85 AC AD 01 30 85 AD AD 02 30 60 E2 02 E3 02 06 30")
    old=analyze(text,blob,{})
    assert old==analyze(text.replace("\n","\r\n"),blob,{})
    assert old["data_roles"]=={"param":2,"local":4}
    assert old["code_buckets"]=={"parameter_home":6,"pointer_area":4,"local_home":3,"return":1}
    assert len(old["motifs"])==1 and old["motifs"][0]["bytes"]==10
    raw={"demo":{"source":"test.ACT","line":1,"pure_asm":False,"raw_menu_data":True}}
    payload=analyze(text,blob,raw)
    assert payload==analyze(text.replace("\n","\r\n"),blob,raw)
    assert payload["code_bytes"]==0 and payload["data_roles"]=={"raw_menu_payload":20}
    assert not payload["motifs"] and not payload["code_buckets"]
    source='; Data for popup menus; each raw byte block ends with $00.\nPROC menu=*()\n[ "Yes" \'Y $00]\nPROC Show()\nPrint(BYTE POINTER(@menu))\nRETURN\n'
    with tempfile.TemporaryDirectory() as directory:
        path=Path(directory)
        for name in ["LIB","DIR","MYDOS","LOCATION","PANELDIR"]:(path/f"{name}.ACT").write_text("")
        (path/"TN.ACT").write_bytes(source.encode());lf=source_inventory(path,"TN")
        (path/"TN.ACT").write_bytes(source.replace("\n","\r\n").encode())
        assert lf==source_inventory(path,"TN")
        assert lf["menu"]["raw_menu_data"] and not lf["show"]["raw_menu_data"]
    audit.check()
    print("Size attribution, frame labels, data/opcode distinction, pointer motif and LF/CRLF checks passed")


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline",type=Path)
    parser.add_argument("--out-dir",type=Path)
    parser.add_argument("--check",action="store_true")
    args=parser.parse_args()
    if args.check:check();return
    assert args.baseline and args.out_dir
    out=args.out_dir;out.mkdir(parents=True,exist_ok=True)
    report={};routine_rows=[];motif_rows=[]
    for program in ["TN","TNDBG"]:
        sources=source_inventory(args.baseline,program)
        for mode in ["optimized","mir6502"]:
            stem=f"{program}-{mode}";asm=args.baseline/f"{stem}.asm";xex=args.baseline/f"{stem}.xex"
            listing=asm.read_text().replace("\r\n","\n");blob=xex.read_bytes();result=analyze(listing,blob,sources)
            assert result==analyze(listing.replace("\n","\r\n"),blob,sources),stem
            assert (args.baseline/f"{stem}-crlf.xex").read_bytes()==blob,stem
            assert result==analyze((args.baseline/f"{stem}-crlf.asm").read_text(),blob,sources),stem
            for name,r in result["routines"].items():
                routine_rows.append(dict(image=stem,routine=name,source=r["source"],line=r["line"],address=f'{r["start"]:04X}',code_bytes=r["code_bytes"],data_bytes=r["data_bytes"],pure_asm=r["pure_asm"],raw_menu_data=r["raw_menu_data"],entry_capture_bytes=r["entry_capture_bytes"],sargs_entry_bytes=r["sargs_entry_bytes"],call_load_transfer_suffix_bytes=r["call_load_transfer_suffix_bytes"],**{k:r["buckets"][k] for k in BUCKETS}))
            for row in result["motifs"]:motif_rows.append(dict(image=stem,**row))
            report[stem]={k:v for k,v in result.items() if k not in ["routines","motifs"]}
            report[stem]["motif_totals"]={kind:{"sites":sum(r["kind"]==kind for r in result["motifs"]),"bytes":sum(r["bytes"] for r in result["motifs"] if r["kind"]==kind)} for kind in ["pair_setup","repeated_pair_setup","boolean_materialization","stored_boolean_materialization","duplicate_literal_byte_exit","word_store_stack_shuttle"]}
            report[stem]["largest_routines"]=[dict(name=n,**{k:v for k,v in r.items() if k!="buckets"}) for n,r in sorted(result["routines"].items(),key=lambda item:-item[1]["code_bytes"])[:15]]
            print(stem,result["load_file_bytes"],result["code_buckets"],report[stem]["motif_totals"],flush=True)
    for name,rows in [("routines.csv",routine_rows),("motifs.csv",motif_rows)]:
        with (out/name).open("w",newline="") as f:
            writer=csv.DictWriter(f,fieldnames=list(rows[0]),lineterminator="\n");writer.writeheader();writer.writerows(rows)
    deltas=[]
    for program in ["TN","TNDBG"]:
        classic={r["routine"]:r for r in routine_rows if r["image"]==f"{program}-optimized"}
        for row in routine_rows:
            if row["image"]!=f"{program}-mir6502" or row["routine"] not in classic:continue
            old=classic[row["routine"]]["code_bytes"];new=row["code_bytes"]
            deltas.append(dict(program=program,routine=row["routine"],classic_code_bytes=old,mir_code_bytes=new,mir_minus_classic=new-old))
    with (out/"backend-deltas.csv").open("w",newline="") as f:
        writer=csv.DictWriter(f,fieldnames=list(deltas[0]),lineterminator="\n");writer.writeheader();writer.writerows(deltas)
    report["provenance"]={"compiler_commit":"2e8aaa06","compiler_sha256":sha((args.baseline.parent/"actionc-qualified").read_bytes()),"source_sha256_lf":{p.name:sha(p.read_text().replace("\r\n","\n").encode()) for p in sorted(args.baseline.glob("*.ACT"))},"analyzer_sha256":sha(Path(__file__).read_bytes()),"byte_parser_sha256":sha(Path(__file__).with_name("audit-current-output.py").read_bytes())}
    (out/"measurements.json").write_text(json.dumps(report,indent=2)+"\n")


if __name__=="__main__":main()
