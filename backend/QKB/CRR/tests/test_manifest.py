"""Manifest stays in sync with the code and the committed copies."""

import json
import sys
import unittest
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "tests"))
from crr import CrrParams, measure_crr  # noqa: E402
from crr.manifest import build  # noqa: E402
from test_geometry import make_tooth, tooth_widths  # noqa: E402


class ManifestTest(unittest.TestCase):
    def test_metric_keys_match_measurement(self):
        documented = {m["key"] for m in build()["algorithms"][0]["outputs"]["metrics"]}
        produced = set(measure_crr(make_tooth(tooth_widths()), CrrParams(pixel_spacing_mm=0.1)).metrics)
        self.assertEqual(documented, produced)

    def test_geometry_and_profile_keys(self):
        out = build()["algorithms"][0]["outputs"]
        m = measure_crr(make_tooth(tooth_widths()))
        self.assertEqual(set(out["geometry"]), set(m.geometry))
        self.assertEqual(set(out["profile"]), set(m.profile))

    def test_params_cover_dataclass(self):
        names = [p["name"] for p in build()["algorithms"][0]["params"]]
        self.assertEqual(names, list(CrrParams.__dataclass_fields__))

    def test_committed_copies_are_current(self):
        expected = json.loads(json.dumps(build()))
        for path in (ROOT / "manifest.json", ROOT.parents[2] / "frontend/src/crr_manifest.json"):
            if path.exists():
                self.assertEqual(json.loads(path.read_text(encoding="utf-8")), expected, f"{path} is stale")
            else:
                self.fail(f"missing {path}; run python -m crr.manifest --out ...")


if __name__ == "__main__":
    unittest.main()
