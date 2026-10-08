import json, sys
S = '/tmp/claude-1000/-home-pals-src-rs-scene-render/6e5c014b-ac81-48e3-835f-be0323e7c6a6/scratchpad/ledger/'
ledger = sys.argv[1]
entries = [json.load(open(f'{S}{i}.json')) for i in range(1, 7)]

def swap(entry, key, old, new):
    assert entry[key].count(old) == 1, (entry['name'], old)
    entry[key] = entry[key].replace(old, new)

swap(entries[4], 'validation', 'the manifest holds 333 documents (326 before)', 'the manifest lists 279 invalid and 47 valid documents (274 and 45 before)')
swap(entries[2], 'validation', 'Balance through the group (ocean_full, 90ce0f1, 2026-10-05, deterministic): worst error',
     'Balance through the group before the pressure of the exchange was credited (ocean_full, 90ce0f1, 2026-10-05, deterministic; the later entries on the pressure of the exchange measure it at 0.00 to 0.09 % of the water): worst error')
swap(entries[2], 'limits', 'so the momentum balance is off by what is in flight (the errors above are 13.2, 10.5 and 4.8 %, falling with the step, not zero)',
     'so the momentum balance is off by what is in flight (the errors above, before the pressure was credited, are 13.2, 10.5 and 4.8 %, falling with the step, not zero)')
d = json.load(open(ledger))
names = {m['name'] for m in d['milestones']}
for e in entries:
    assert e['name'] not in names
    assert set(e) == {'name', 'evidence', 'validation', 'limits'}
d['milestones'].extend(entries)
open(ledger, 'w').write(json.dumps(d, indent=2, ensure_ascii=False) + '\n')
print(len(d['milestones']))
