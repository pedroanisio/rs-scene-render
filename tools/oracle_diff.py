#!/usr/bin/env python3
"""Differential test of `scene-render validate` against lxml (tools/oracle.py).

Mutates the kitchen-sink document at random (attribute deletion, value
replacement, attribute insertion, element deletion, duplication, renaming and
moves), then compares, for every mutant:

* the XSD verdict: invalid for lxml <=> a structural error (S01–S09, S11, S12)
  from scene-render. S10 and whitespace-only IDREFS are excluded because
  libxml2 does not check them. libxml2 also accepts `1e` (exponent without
  digits) as an xs:double, which XSD 1.0 forbids; the value pool omits it;
* the Schematron result: the multiset of (assert id, line) pairs.

Usage: tools/oracle_diff.py [COUNT] [SEED]   (needs a built target/release or target/debug binary)
"""
import collections, copy, json, random, subprocess, sys, tempfile
from pathlib import Path

from lxml import etree

sys.path.insert(0, str(Path(__file__).parent))
from oracle import verdict

ROOT = Path(__file__).resolve().parent.parent
BIN = next(p for p in [ROOT / "target/release/scene-render", ROOT / "target/debug/scene-render"] if p.exists())
BASE = (ROOT / "tests/corpus/valid/kitchen-sink.scene.xml").read_bytes()

def pools(tree):
    els = [e for e in tree.iter() if isinstance(e.tag, str)]
    names = sorted({e.tag for e in els})
    attrs = sorted({a for e in els for a in e.attrib})
    ids = sorted({e.get("id") for e in els if e.get("id")})
    values = ["", " ", "0", "1", "-1", "1.5", "2.5e1", "+1", "NaN", "INF", "abc", "true", "false",
              "#ff0000", "1,0,0", "var(--bg)", "var(--nope)", "url(#sunset)", "url(#nope)", "50%", "12vw",
              "30000/1001", "path", "svg", "pin", "spring", "shader", "luma", "track", "follow-path", "skin",
              "corner-pin", "steps", "cubic-bezier", "speech", "music", "prores", "h264", "custom", "list",
              "0 0 1 0 1 1 0 1", "glow-fx grade", "key dome"] + ids
    return els, names, attrs, values

def mutate(rng, tree):
    els, names, attrs, values = pools(tree)
    root = tree.getroot()
    for _ in range(rng.randint(1, 3)):
        els = [e for e in root.iter() if isinstance(e.tag, str)]
        e = rng.choice(els)
        op = rng.randrange(7)
        if op == 0 and e.attrib:
            del e.attrib[rng.choice(list(e.attrib))]
        elif op == 1 and e.attrib:
            e.set(rng.choice(list(e.attrib)), rng.choice(values))
        elif op == 2:
            e.set(rng.choice(attrs), rng.choice(values))
        elif op == 3 and e is not root:
            e.getparent().remove(e)
        elif op == 4 and e is not root:
            e.addnext(copy.deepcopy(e))
        elif op == 5 and e is not root:
            e.tag = rng.choice(names)
        elif op == 6 and e is not root:
            target = rng.choice(els)
            if e not in target.iterancestors() and target is not e:
                e.getparent().remove(e)
                target.append(e)
    return etree.tostring(tree, xml_declaration=True, encoding="UTF-8")

def main():
    count = int(sys.argv[1]) if len(sys.argv) > 1 else 1000
    seed = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    rng = random.Random(seed)
    tmp = Path(tempfile.mkdtemp(prefix="sr-diff-"))
    docs = []
    for i in range(count):
        data = mutate(rng, etree.ElementTree(etree.fromstring(BASE)))
        p = tmp / f"m{i:05d}.xml"
        p.write_bytes(data)
        docs.append((p, data))
    out = subprocess.run([str(BIN), "validate", "--no-assets", "--format", "json", *[str(p) for p, _ in docs]],
                         capture_output=True, text=True)
    report = {f["file"]: f for f in json.loads(out.stdout)["files"]}
    mismatches = 0
    stats = collections.Counter()
    for p, data in docs:
        xsd, sch = verdict(data)
        diags = report[str(p)]["diagnostics"]
        rust_struct = [d for d in diags if d["code"].startswith("S") and d["code"] != "S10" and d["severity"] == "error"]
        blind = any(d["code"] == "S06" and "at least one" in d["message"] for d in rust_struct)
        rust_struct_invalid = bool([d for d in rust_struct if not (d["code"] == "S06" and "at least one" in d["message"])])
        rust_sch = collections.Counter((d["code"], d["loc"]["line"]) for d in diags if d["code"][0] in "VCR")
        ora_sch = collections.Counter(sch)
        xsd_ok = (bool(xsd) == rust_struct_invalid) or (blind and not xsd)
        stats["xsd-invalid" if xsd else "xsd-valid"] += 1
        stats["sch-failures"] += sum(ora_sch.values())
        if not xsd_ok or rust_sch != ora_sch:
            mismatches += 1
            if mismatches <= 10:
                print(f"MISMATCH {p}")
                if not xsd_ok:
                    print("  lxml xsd:", xsd[:3])
                    print("  rust    :", [(d["code"], d["loc"]["line"], d["message"]) for d in rust_struct][:3])
                if rust_sch != ora_sch:
                    print("  lxml sch only:", dict(ora_sch - rust_sch))
                    print("  rust sch only:", dict(rust_sch - ora_sch))
    print(f"{count} mutants, seed {seed}: {dict(stats)}; mismatches: {mismatches}")
    return 1 if mismatches else 0

if __name__ == "__main__":
    sys.exit(main())
