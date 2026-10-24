#!/usr/bin/env python3
"""Read-only dev1 audit after both paid lots have become physical machinery."""
import json,subprocess,time
from pathlib import Path
root=Path('/opt/rustmush/dev1');out=root/'r9a-evidence'
assert root.resolve()==root

def read(k):return json.loads(subprocess.check_output(['redis-cli','-p','6788','--raw','JSON.GET',k],text=True) or 'null')
def docs(pattern):
 ks=subprocess.check_output(['redis-cli','-p','6788','--scan','--pattern',pattern],text=True).splitlines();assert len(ks)<10000
 return [read(k) for k in ks]
f=read('mod:tacsim:tacsim:relay_foundry');assert len(f['machines'])==2
books=[d['value'] for d in docs('mod:tacsim:action-state/v1/tacsim.relay_fabrications/*') if d['value']['facility']==f['facility']];assert len(books)==1
book=books[0]
assert book['stocked'] and len(book['receipts'])==2
materials={}
for doc in docs('mod:inventory:action-state/v1/inventory_books/*'):
 b=doc['value'];r=b['resource']
 if r['subject']['id']!='v'+str(f['vault']):continue
 code=r['id'];scalar=read('mod:inventory:action-state/v1/inventory_pools/'+b['id'])['value']
 materials[code]={'scalar':scalar,'reservations':b}
for code,remaining,consumed in [('relay_blank',6,2),('instrument_boards',0,4)]:
 m=materials[code];assert m['scalar']['current']==remaining
 assert not any(r['state']['kind']=='held' for r in m['reservations']['records'])
 assert sum(r['amount'] for r in m['reservations']['records'] if r['state']['kind']=='consumed')==consumed
lots=[d['value'] for d in docs('mod:inventory:action-state/v1/inventory_lots/*')]
assert not any(l['endpoint']==f['vault'] and l['commodity']=='relay_pair' for l in lots)
assert not any(l['id']==r['lot'] for l in lots for r in book['receipts'].values())
orders=[d['value'] for d in docs('mod:industry:action-state/v1/industry_work_orders/*') if str(d['value']['id']) in book['receipts']]
assert len(orders)==2 and all(o['state']['kind']=='completed' and o['schedule']['progress_units']==120 for o in orders)
holds=[h for h in docs('mod:economy:hold:*') if h['purpose'].get('Reservation',{}).get('kind')=='industry.workorder' and str(h['purpose']['Reservation']['id']) in book['receipts']]
assert not any(h['status']=='Held' for h in holds)
assert sum(isinstance(h['status'],dict) and 'Consumed' in h['status'] for h in holds)==2
halves=[h for m in f['machines'].values() for h in m['halves']];assert len(set(halves))==4
result={'passed':True,'at':time.time(),'materials':materials,'fabrications':book,'orders':orders,'budgetHolds':holds,'halves':halves,'outputLotsRemaining':0,'note':'Captured after both real work orders settled and their unique output lots were consumed; all Redis access was read-only.'}
live=json.loads((out/'live.json').read_text())
ticket=next(m['ticket'] for m in live['milestones'] if m['test']=='departure-deimos')
manifest=ticket['manifest']
goods={g['id']:g for g in json.loads((root/'build/crates/tacsim/content/market/goods.json').read_text())['goods']}
logistics=json.loads((root/'build/crates/tacsim/content/units/dray_p.json').read_text())['entries']['tacsim.unit.dray_p']['def']['logistics']
base=manifest['occupants']*logistics['occupant_allowance_kg']+manifest['tank']*goods['propellant']['measures']['si.mass_kg']+manifest['locker']*goods['provisions']['measures']['si.mass_kg']+sum(q*goods[g]['measures']['si.mass_kg'] for g,q in manifest['cargo'].items())
assert ticket['payload_kg']-base==10000
result['loadedPassage']={'ticket':ticket,'nonRelayPayloadKg':base,'relayCargoKg':ticket['payload_kg']-base,'actualProcessDeath':next(m for m in live['milestones'] if m['test']=='two-titled-halves-in-real-passage')}
(out/'material-audit.json').write_text(json.dumps(result,indent=2)+'\n')
print('R9A_FINITE_MATERIALS_AND_TWO_REAL_PAYMENTS_PASSED')
