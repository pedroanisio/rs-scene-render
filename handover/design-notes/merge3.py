"""Three-way merge of the named top-level components of the schema files, on top of the upstream file."""
import sys
from lxml import etree

def comps(path, key):
    """ordered list of (key, text-block, c14n) of the top-level children of the root that `key` names."""
    text = open(path).read().split('\n')
    tree = etree.parse(path)
    root = tree.getroot()
    kids = [k for k in root if isinstance(k.tag, str)]
    starts = [k.sourceline for k in kids]
    out = []
    for i, k in enumerate(kids):
        nm = key(k)
        # a block starts at the line of the element (comments before it stay with the previous block) and runs to the line before the next element
        first = starts[i] - 1
        last = (starts[i + 1] - 1) if i + 1 < len(kids) else None
        # extend the end of the element to its closing tag line: the next element's start minus blank/comment lines is kept with this block
        block = text[first:last]
        out.append((nm, '\n'.join(block), etree.tostring(k, method='c14n').decode()))
    head = '\n'.join(text[:starts[0] - 1])
    tail = '\n'.join(text[kids and (kids[-1].sourceline - 1):]) if False else ''
    return head, out

def xsd_key(k):
    tag = etree.QName(k).localname
    return (tag, k.get('name') or k.get('ref') or '')

def sch_key(k):
    tag = etree.QName(k).localname
    return (tag, k.get('id') or k.get('prefix') or k.get('uri') or '')

def merge(base, ours, theirs, key, label):
    bh, bc = comps(base, key); oh, oc = comps(ours, key); th, tc = comps(theirs, key)
    B = {k: c for k, _, c in bc}; O = {k: (t, c) for k, t, c in oc}; T = {k: (t, c) for k, t, c in tc}
    report = {'ours': [], 'theirs': [], 'conflict': [], 'added_ours': [], 'dropped_by_theirs': []}
    result = []
    for k, t, c in tc:
        if k in O and k in B:
            ot, oc_ = O[k]
            if c == B[k]:
                if oc_ != B[k]:
                    result.append((k, ot)); report['ours'].append(k); continue
            elif oc_ == B[k]:
                result.append((k, t)); report['theirs'].append(k); continue
            elif oc_ == c:
                result.append((k, t)); continue
            else:
                report['conflict'].append(k); result.append((k, t)); continue
        elif k in O and k not in B:
            if O[k][1] != c:
                report['conflict'].append(k)
            result.append((k, t)); continue
        result.append((k, t))
    present = {k for k, _ in result}
    # components that only we have (added since the base or kept) go after the one that precedes them in our file
    prev = None
    order_o = [k for k, _, _ in oc]
    for i, k in enumerate(order_o):
        if k in present or k in T:
            prev = k; continue
        if k in B and k not in T:
            report['dropped_by_theirs'].append(k); continue   # upstream removed it on purpose?
        report['added_ours'].append(k)
        idx = next((j for j, (kk, _) in enumerate(result) if kk == prev), len(result) - 1)
        result.insert(idx + 1, (k, O[k][0])); present.add(k); prev = k
    return th, result, report

if __name__ == '__main__':
    kind, base, ours, theirs, out = sys.argv[1:6]
    key = xsd_key if kind == 'xsd' else sch_key
    head, result, report = merge(base, ours, theirs, key, kind)
    # the closing tag of the root
    t = open(theirs).read().split('\n')
    close = [l for l in t if l.strip().startswith('</')][-1]
    open(out, 'w').write(head + '\n' + '\n'.join(b for _, b in result) + '\n' + close + '\n')
    for k, v in report.items():
        print(k, len(v), v[:60])
