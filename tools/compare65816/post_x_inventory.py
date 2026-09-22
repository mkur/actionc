#!/usr/bin/env python3
"""Roll up retained word stores and unconditional backedge loads, by actual PCs."""
import argparse
import json
from pathlib import Path
from report import digest


def movement(inventory):
    builds=[]
    for b in inventory['builds']:
        sites=[]
        for r in b['routines']:
            if not r['counted']:continue
            latches={loop['latch'] for loop in r['loops']}
            for i in r['instructions']:
                groups=[]
                op=bytes.fromhex(i['bytes'])[0]
                if op in (0x83,0x85) and i['a_bytes']==2:
                    groups.append('retained_word_stores')
                    if i['memory'][0]['owner']['kind']=='private_temp':groups.append('private_temp_word_stores')
                if 'word_load_cycles' in i and i['scope']['kind']=='goto' and i['scope']['block'] in latches:
                    groups.append('goto_backedge_word_loads')
                if groups:sites.append(dict(pc=i['pc'],bytes=i['bytes'],scope=i['scope'],memory=i['memory'],groups=groups))
        records=[]
        for r in b['measurements']:
            counts={g:sum(r['instruction_sites'].get(str(i['pc']),0) for i in sites if g in i['groups'])
                    for g in ('retained_word_stores','private_temp_word_stores','goto_backedge_word_loads')}
            counts.update(vector=r['vector'],traffic={k:v for k,v in r['traffic'].items() if ':edge_staging:' in k or ':mutable_parameter:' in k})
            records.append(counts)
        builds.append(dict(case=b['case'],mode=b['mode'],sites=sites,records=records))
    return dict(schema=1,scope='All counted emitted word STA sites, including required stores; unconditional natural-backedge word LDA sites, including staging captures/restores. Counts do not imply redundancy.',builds=builds)


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('inventory',type=Path);p.add_argument('--output',type=Path,required=True);p.add_argument('--check',action='store_true');a=p.parse_args()
    result=movement(json.loads(a.inventory.read_text()));result['inventory_sha256']=digest(a.inventory);content=json.dumps(result,indent=2)+'\n'
    if a.check:assert a.output.read_text()==content
    else:a.output.write_text(content)
    print('Checked 28 builds of retained stores, backedge loads, staging and mutable traffic')
