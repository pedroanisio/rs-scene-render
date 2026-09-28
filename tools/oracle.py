"""Reference validator: lxml's XSD 1.0 validator plus ISO Schematron (libxslt).

Usage as a module: `verdict(xml_bytes) -> (xsd_errors, sch_failures)` where
xsd_errors is a list of (line, message) and sch_failures a list of (assert id, line).
"""
import sys
from pathlib import Path
from lxml import etree, isoschematron

ROOT = Path(__file__).resolve().parent.parent
_XSD = etree.XMLSchema(etree.parse(str(ROOT / "schema" / "scene-render-1.1.xsd")))
_SCH = isoschematron.Schematron(etree.parse(str(ROOT / "schema" / "scene-render-1.1.sch")), store_report=True)
SVRL = "{http://purl.oclc.org/dsdl/svrl}"

def verdict(data: bytes):
    try:
        doc = etree.fromstring(data, etree.XMLParser(resolve_entities=False, no_network=True)).getroottree()
    except etree.XMLSyntaxError as e:
        return [(e.lineno, "XML: " + str(e))], []
    _XSD.validate(doc)
    xsd = [(e.line, e.message) for e in _XSD.error_log]
    _SCH.validate(doc)
    fails = []
    for fa in _SCH.validation_report.getroot().iter(SVRL + "failed-assert"):
        loc = fa.get("location")
        try:
            node = doc.xpath(loc)[0]
            line = node.sourceline
        except Exception:
            line = 0
        fails.append((fa.get("id"), line))
    return xsd, fails

if __name__ == "__main__":
    for p in sys.argv[1:]:
        xsd, sch = verdict(Path(p).read_bytes())
        print(p, "valid" if not xsd and not sch else "INVALID")
        for l, m in xsd:
            print(f"  xsd {l}: {m}")
        for i, l in sch:
            print(f"  sch {i} line {l}")
