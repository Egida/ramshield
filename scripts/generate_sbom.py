#!/usr/bin/env python3
import argparse, hashlib, json, subprocess
from datetime import datetime, timezone
p=argparse.ArgumentParser(); p.add_argument('--version',required=True); p.add_argument('-o','--output',required=True); a=p.parse_args()
meta=json.loads(subprocess.check_output(['cargo','metadata','--locked','--format-version','1'],text=True))
pkgs=meta['packages']; pkgs=sorted(pkgs,key=lambda x:(x['name'],x['version'],x['id']))
def sid(p): return 'SPDXRef-'+hashlib.sha256(p['id'].encode()).hexdigest()[:20]
by_id={p['id']:p for p in pkgs}
root=next((p for p in pkgs if p['name']=='ramshield'),pkgs[0]); created=datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace('+00:00','Z')
out={'spdxVersion':'SPDX-2.3','dataLicense':'CC0-1.0','SPDXID':'SPDXRef-DOCUMENT','name':f'ramshield-{a.version}','documentNamespace':f'https://github.com/grep999/ramshield/spdx/{a.version}/{hashlib.sha256(created.encode()).hexdigest()}','creationInfo':{'created':created,'creators':['Tool: RamShield SBOM generator']},'packages':[],'relationships':[]}
for p in pkgs:
  lic=p.get('license') or 'NOASSERTION'; src=p.get('source') or f'local:{p["name"]}'
  out['packages'].append({'SPDXID':sid(p),'name':p['name'],'versionInfo':p['version'],'downloadLocation':src,'licenseConcluded':lic,'licenseDeclared':lic,'filesAnalyzed':False})
for node in meta.get('resolve',{}).get('nodes',[]):
  for dep in node.get('deps',[]):
    target=dep.get('pkg')
    if node.get('id') in by_id and target in by_id:
      out['relationships'].append({'spdxElementId':sid(by_id[node['id']]),'relationshipType':'DEPENDS_ON','relatedSpdxElement':sid(by_id[target])})
with open(a.output,'w') as f: json.dump(out,f,indent=2,sort_keys=True); f.write('\n')
