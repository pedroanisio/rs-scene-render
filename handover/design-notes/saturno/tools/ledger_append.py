"""ledger_append.py SOURCE_REV NAME_PREFIX: appends to the ledger of the work tree the milestone whose name starts with NAME_PREFIX in tools/evidence/cinematic-impact.json at SOURCE_REV
(textually, as the entries are written, so that the others are not reformatted), or replaces it if it is there. Refuses repeated keys."""
import json, subprocess, sys
rev, prefix = sys.argv[1], sys.argv[2]
p = 'tools/evidence/cinematic-impact.json'
src = subprocess.run(['git', 'show', f'{rev}:{p}'], capture_output=True, text=True, check=True).stdout
entry = [m for m in json.loads(src)['milestones'] if m.get('name', '').startswith(prefix)][-1]
raw = open(p).read()
def j(x): return json.dumps(x, ensure_ascii=False)
text = '    {\n      "name": ' + j(entry['name']) + ',\n      "evidence": [\n' + ',\n'.join('        ' + j(e) for e in entry['evidence']) + '\n      ],\n      "validation": ' + j(entry['validation']) + ',\n      "limits": ' + j(entry['limits']) + '\n    }'
dec = json.JSONDecoder()
start = raw.index('"milestones": [') + len('"milestones": [')
import re
pos = start; spans = []
ws = re.compile(r'\s*')
while True:
    pos = ws.match(raw, pos).end()
    if raw[pos] == ']': break
    obj, end = dec.raw_decode(raw, pos)
    spans.append((pos, end, obj))
    pos = ws.match(raw, end).end()
    if raw[pos] == ',': pos += 1
close = pos
same = [s for s in spans if s[2].get('name', '').startswith(prefix)]
if same:
    s0, e0, _ = same[-1]
    out = raw[:s0] + text.lstrip() + raw[e0:]
else:
    last_end = spans[-1][1]
    out = raw[:last_end] + ',\n' + text + raw[last_end:]
def hook(pairs):
    keys = [k for k, _ in pairs]
    assert len(keys) == len(set(keys)), keys
    return dict(pairs)
d = json.loads(out, object_pairs_hook=hook)
assert all(all(k in m for k in ('name', 'evidence', 'validation', 'limits')) for m in d['milestones'])
open(p, 'w').write(out)
print(len(d['milestones']), 'milestones')
