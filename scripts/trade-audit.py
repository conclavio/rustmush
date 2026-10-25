#!/usr/bin/env python3
"""Read-only audit of the isolated live trade store; run on the dev1 VPS.

Snapshots contain only trade/economy state, never auth records. Run before and
following each acceptance phase. No Redis writes, clock changes or player login.
"""
import datetime
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib
from urllib.parse import urlparse

ROOT = Path('/opt/rustmush/dev1/trade-acceptance')
label = sys.argv[1]
assert re.fullmatch(r'[a-z0-9-]+', label)
config = tomllib.loads((ROOT / 'mushd.toml').read_text())
url = urlparse(config['store']['redis_url'])
assert url.hostname == '127.0.0.1' and url.port == 6798
assert config['game']['ic_ratio'] == 24
assert config['tacsim']['time']['ephemeris']['rate'] == 0
cli = ['redis-cli', '-p', str(url.port), '-n', url.path.strip('/') or '0', '--raw']

def redis(*args):
    return subprocess.check_output(cli + list(args), text=True, timeout=30).strip()

patterns = ['mod:economy:account:*', 'mod:economy:contract:*', 'mod:economy:vault:*', 'mod:economy:escalation:*',
            'mod:tacsim:tacsim:port:*', 'mod:tacsim:tacsim:orders:*',
            'mod:tacsim:tacsim:voyage:*', 'mod:tacsim:tacsim:notice:*',
            'mod:tacsim:action-state/v1/tacsim.port_stock/*',
            'mod:tacsim:action-state/v1/tacsim.ticket/*',
            'mod:inventory:action-state/v1/inventory_*/*',
            'mod:sim:cjourney:*', 'mod:rustmush_kernel_time:world_ic/*']

def keys():
    return sorted({key for pattern in patterns for key in redis('--scan', '--pattern', pattern).splitlines()})

for attempt in range(4):
    names = keys()
    assert 0 < len(names) < 4096, 'bounded acceptance fixture'
    # One JSON.MGET is atomic; retry if a new relevant key appeared meanwhile.
    values = [json.loads(line) for line in redis('JSON.MGET', *names, '.').splitlines()]
    if names == keys() and all(value is not None for value in values):
        break
else:
    raise AssertionError('trade store did not yield a consistent key census')
data = dict(zip(names, values, strict=True))
now = datetime.datetime.now(datetime.UTC).isoformat()
(ROOT / 'evidence' / (label + '-snapshot.json')).write_text(json.dumps(data, indent=2) + '\n')

def family(prefix):
    return [value for key, value in data.items() if key.startswith(prefix)]

def typed(name):
    return {value['key']: value['value'] for key, value in data.items()
            if ':action-state/v1/' in key
            and value.get('family', {}).get('id') == name}

accounts = family('mod:economy:account:')
credits = [a for a in accounts if a['instrument'] == 'tacsim.credit']
assert sum(a['posted_balance'] for a in credits) == 0, 'closed double-entry credit ledger'
ports = family('mod:tacsim:tacsim:port:')
issued_credit = sum(c['amount_minor'] for p in ports for org in p.get('orgs', {}).values() for c in org['capital'])
treasury = [a for a in credits if a['owner'] == {'System': 'tacsim.treasury'}]
assert len(treasury) == 1 and treasury[0]['posted_balance'] == -issued_credit, 'only the recorded one-shot capital was issued'
row = next(h for p in ports for h in p.get('hulls', []) if h['mark'] == 'FPC-D-0401')
hull = row['hull']
vaults = [v['id'] for v in family('mod:economy:vault:') if v['backing'] == {'SpatialInterior': row['rooms']['stowage']}]
# The hold vault is created by enrolment, not by lane provisioning.
assert len(vaults) == 1 or (label == 'before-day1' and not vaults), 'one enrolled hull hold'
vault = vaults[0] if vaults else None
books, pools = typed('inventory_books'), typed('inventory_pools')
quantities = {b['resource']['id']: pools[k]['current'] for k, b in books.items()
              if b['resource']['subject']['id'] == 'v' + str(vault)}
goods = {g['id']: g for g in json.loads((ROOT.parent / 'build/crates/tacsim/content/market/goods.json').read_text())['goods']}
unit = json.loads((ROOT.parent / 'build/crates/tacsim/content/units/dray.json').read_text())

def stowage(value):
    if isinstance(value, dict):
        if 'dry_mass_t' in value and 'stowage' in value:
            return value['stowage']
        for child in value.values():
            found = stowage(child)
            if found is not None:
                return found
    return None

loads = {}
for compartment, rule in stowage(unit).items():
    load = {measure: sum(q * goods[g]['measures'].get(measure, 0) for g, q in quantities.items()
                         if goods[g]['stowage'] in rule['accepts']) for measure in rule['limits']}
    assert all(load[m] <= ceiling for m, ceiling in rule['limits'].items()), (compartment, load, rule)
    loads[compartment] = {'load': load, 'limits': rule['limits']}
tranches = {t['id']: t for t in typed('inventory_tranches').values()}
origins = {s['lot'] for s in typed('tacsim.port_stock').values()}

def seeded(tranche, seen):
    assert tranche['id'] not in seen, 'acyclic lineage'
    seen = seen | {tranche['id']}
    if set(tranche['absorbed']) & origins:
        return True
    parents = tranche['inherited'] + ([tranche['predecessor']] if tranche['predecessor'] is not None else [])
    return bool(parents) and all(p in tranches and seeded(tranches[p], seen) for p in parents)

lineage = {}
for good, qty in quantities.items():
    if qty == 0:
        continue
    roots = [t for t in tranches.values() if t['endpoint'] == vault and t['commodity'] == good]
    assert roots, ('missing lineage', good)
    current = max(roots, key=lambda t: t['generation'])
    assert seeded(current, set()), ('unseeded goods', good)
    lineage[good] = {'quantity': qty, 'seeded': True}
seed_totals = {}
for stock in typed('tacsim.port_stock').values():
    seed_totals[stock['good']] = seed_totals.get(stock['good'], 0) + stock['qty']
burns = list(typed('tacsim.ticket').values())
goods_conservation = {}
for good, seeded_qty in seed_totals.items():
    held = sum(pools[key]['current'] for key, book in books.items() if book['resource']['id'] == good)
    consumed = sum(ticket['plan']['burn_t' if good == 'propellant' else 'crew_days'] for ticket in burns) if good in ['propellant', 'provisions'] else 0
    assert held + consumed == seeded_qty, ('goods conservation', good, held, consumed, seeded_qty)
    goods_conservation[good] = {'seeded': seeded_qty, 'held': held, 'burned': consumed}
assert not family('mod:economy:escalation:'), 'no settlement or stale-contract escalation'
contracts = family('mod:economy:contract:')
def contract_state(contract):
    state = contract['state']
    return state if isinstance(state, str) else next(iter(state))
refused = [c['id'] for c in contracts if contract_state(c) in ['SettlementRefused', 'Disputed']]
assert not refused, ('refused/disputed contracts', refused)
clock = family('mod:rustmush_kernel_time:world_ic/epoch/')
assert len(clock) == 1 and clock[0]['rate'] == 24, 'unchanged rate-24 timeline'
orders = data.get('mod:tacsim:tacsim:orders:' + str(hull))
tickets = [t for t in typed('tacsim.ticket').values() if t['hull'] == hull]
result = {'at': now, 'passed': True, 'database': int(url.path.strip('/') or '0'),
          'hull': hull, 'vault': vault, 'rate': 24, 'clock_generations': len(clock),
          'credit_accounts': len(credits), 'credit_total_minor': 0, 'issued_credit_minor': issued_credit,
          'hold': quantities, 'compartments': loads, 'lineage': lineage, 'goods_conservation': goods_conservation, 'escalations': 0,
          'ticket_count': len(tickets), 'orders_cursor': orders and orders['cursor'],
          'contract_states': {state: sum(contract_state(c) == state for c in contracts) for state in sorted({contract_state(c) for c in contracts})}}
(ROOT / 'evidence' / (label + '-audit.json')).write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result))
