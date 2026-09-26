"""Run: python -m unittest discover -s tests   (from backend/QKB/CRR; numpy only)"""

import json
import sys
import unittest
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from crr import STATUS_INVALID_MASK, STATUS_NO_NECK, STATUS_OK, CrrParams, measure_crr  # noqa: E402


def make_tooth(widths, height=None, canvas_w=200, cx=100):
    """Vertical mask; row i has centered width widths[i]."""
    mask = np.zeros((len(widths), canvas_w), dtype=bool)
    for y, w in enumerate(widths):
        half = w / 2
        mask[y, int(round(cx - half)) : int(round(cx + half))] = True
    return mask


def tooth_widths(crown=150, root=150):
    """wide crown (60) -> neck (36) -> tapering root (50 -> 4)."""
    crown_w = [60] * crown
    root_w = list(np.linspace(50, 4, root).round().astype(int))
    neck_w = [36] * 10
    return crown_w[: crown - 5] + neck_w + root_w[5:]


class MeasureCrrTest(unittest.TestCase):
    def test_found_ratio_close_to_geometry(self):
        m = measure_crr(make_tooth(tooth_widths(150, 150)))
        self.assertEqual(m.status, STATUS_OK)
        self.assertAlmostEqual(m.ratio, 1.0, delta=0.15)
        self.assertGreater(m.metrics["neck_prominence"], 0.04)
        self.assertLess(m.metrics["neck_width_rel"], 0.8)
        # crown end is the wide (top) end
        self.assertLess(m.geometry["crown_tip"][1], m.geometry["neck"][1])
        self.assertGreater(m.geometry["root_apex"][1], m.geometry["neck"][1])

    def test_ratio_follows_lengths(self):
        m = measure_crr(make_tooth(tooth_widths(200, 100)))
        self.assertEqual(m.status, STATUS_OK)
        self.assertAlmostEqual(m.ratio, 2.0, delta=0.3)

    def test_orientation_invariant(self):
        base = make_tooth(tooth_widths(180, 120))
        r0 = measure_crr(base).ratio
        r_flip = measure_crr(base[::-1]).ratio
        r_T = measure_crr(base.T).ratio
        self.assertAlmostEqual(r0, r_flip, delta=0.05)
        self.assertAlmostEqual(r0, r_T, delta=0.05)

    def test_monotone_taper_is_not_a_neck(self):
        widths = list(np.linspace(60, 4, 300).round().astype(int))
        m = measure_crr(make_tooth(widths))
        self.assertEqual(m.status, STATUS_NO_NECK)
        self.assertIsNone(m.ratio)
        self.assertIsNotNone(m.geometry["axis_extent"])

    def test_tiny_mask_invalid(self):
        mask = np.zeros((50, 50), dtype=bool)
        mask[10:12, 10:12] = True
        m = measure_crr(mask)
        self.assertEqual(m.status, STATUS_INVALID_MASK)

    def test_pixel_spacing_and_json(self):
        m = measure_crr(make_tooth(tooth_widths()), CrrParams(pixel_spacing_mm=0.1))
        d = json.loads(json.dumps(m.to_dict(), allow_nan=False))
        self.assertAlmostEqual(d["metrics"]["crown_length_mm"], d["metrics"]["crown_length_px"] * 0.1, places=2)
        self.assertEqual(len(d["profile"]["t"]), len(d["profile"]["width_smooth"]))

    def test_invalid_params(self):
        with self.assertRaises(ValueError):
            measure_crr(np.ones((10, 10), bool), CrrParams(n_bins=3))


if __name__ == "__main__":
    unittest.main()
