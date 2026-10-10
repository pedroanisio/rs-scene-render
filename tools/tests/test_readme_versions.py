"""The README documents every feature of document version 1.6, each with an example that parses."""
import re
import sys
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))

# section heading -> names its text must mention
FEATURES = {
    "Compute nodes": ["<compute>", "sr_point", "sr_accumulate", "invocations", "fractionBits", "CMP12"],
    "Tonemaps": ["<tonemap>", "reference", "ramp", "gamma", "alpha"],
    "Iterate": ["<iterate>", "until=\"converged\"", "checkEvery", "tolerance", "ITR1"],
    "Programs": ["<program>", "generate", "init", "step", "frame", "fuel", "memoryLimit", "outputSha256"],
    "Steps per frame and prewarm": ["stepsPerFrame", "prewarm", "FRAMEINDEX", "TIMEDELTA", "STP1"],
    "Parametric paths": ["parametricPath", "samples", "closed"],
    "Parametric surfaces and heightfields": ["parametricSurface", "heightfield", "uSamples", "xSamples"],
    "Pixel-exact options": ["precision", "sr_ContentRect", "edgeBlend", "supersample"],
    "Flocks that stay upright": ["orientToVelocity", "INERT-I16"],
}


def section(text, title, level):
    start = re.search(rf"^{'#' * level} {re.escape(title)}\n", text, re.M)
    if not start:
        return None
    end = re.compile(rf"^#{{1,{level}}} ", re.M).search(text, start.end())
    return text[start.end():end.start() if end else len(text)]


class Version16ReadmeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.v16 = section((ROOT / "README.md").read_text(encoding="utf-8"), "Document version 1.6", 2)

    def test_every_feature_has_a_section_with_an_example(self):
        self.assertIsNotNone(self.v16, "README.md has no '## Document version 1.6' section")
        for title, names in FEATURES.items():
            body = section(self.v16, title, 3)
            self.assertIsNotNone(body, f"no '### {title}'")
            for name in names:
                self.assertIn(name, body, f"'### {title}' does not mention {name}")
            self.assertIn("```", body, f"'### {title}' has no example")

    def test_the_xml_examples_parse_and_whole_documents_validate(self):
        self.assertIsNotNone(self.v16)
        from oracle import verdict
        blocks = re.findall(r"```xml\n(.*?)```", self.v16, re.S)
        self.assertGreaterEqual(len(blocks), len(FEATURES))
        for xml in blocks:
            if xml.lstrip().startswith("<scene"):
                xsd, sch = verdict(xml.encode())
                self.assertEqual((xsd, sch), ([], []), xml)
            else:
                ET.fromstring(f"<fragment>{xml}</fragment>")


if __name__ == "__main__":
    unittest.main()
