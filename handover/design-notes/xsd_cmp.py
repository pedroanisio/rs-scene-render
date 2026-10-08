import sys
from lxml import etree
ns={'x':'http://www.w3.org/2001/XMLSchema'}
def load(p):
    t=etree.parse(p)
    types={}
    for ct in t.xpath('/x:schema/x:complexType|/x:schema/x:attributeGroup|/x:schema/x:simpleType|/x:schema/x:group',namespaces=ns):
        name=ct.get('name'); kind=etree.QName(ct).localname
        attrs={}
        for a in ct.xpath('.//x:attribute',namespaces=ns):
            n=a.get('name') or a.get('ref')
            # canonical form of the attribute definition
            d=dict(a.attrib)
            st=a.xpath('x:simpleType',namespaces=ns)
            d['_simple']=etree.tostring(st[0],method='c14n').decode() if st else ''
            attrs[n]=d
        elems=sorted(set(e.get('name') or e.get('ref') for e in ct.xpath('.//x:element',namespaces=ns) if e.get('name') or e.get('ref')))
        types[(kind,name)]=(attrs,elems,etree.tostring(ct,method='c14n').decode())
    return types
up=load(sys.argv[1]); our=load(sys.argv[2])
print('types only in up:',sorted(k for k in up if k not in our))
print('types only in ours:',sorted(k for k in our if k not in up))
for k in sorted(set(up)&set(our)):
    ua,ue,uc=up[k]; oa,oe,oc=our[k]
    if uc==oc: continue
    only_up=sorted(set(ua)-set(oa)); only_our=sorted(set(oa)-set(ua))
    diff=[a for a in set(ua)&set(oa) if {x:y for x,y in ua[a].items()}!={x:y for x,y in oa[a].items()}]
    el=(sorted(set(ue)-set(oe)),sorted(set(oe)-set(ue)))
    print(k,'| attrs only up:',only_up,'| only ours:',only_our,'| differing:',sorted(diff),'| elems up-only/our-only:',el if el!=([],[]) else '-')
