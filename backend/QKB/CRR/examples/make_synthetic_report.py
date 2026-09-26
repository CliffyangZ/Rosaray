"""Build a synthetic tilted tooth and write a viewer report (no OpenCV/SAM needed).

    python examples/make_synthetic_report.py examples/synthetic_report.json
"""

import json
import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from crr import measure_crr  # noqa: E402
from crr.report import REPORT_SCHEMA  # noqa: E402

H, W, ANGLE = 640, 420, np.radians(12)


def half_width(y):  # y: 0 (crown tip) .. 500 (root apex); rounded crown, neck at ~280, tapering root
    if y < 40:
        return 34 * np.sqrt(max(1 - ((40 - y) / 40) ** 2, 0)) + 8
    if y < 250:
        return 42 - 2 * np.sin(np.pi * (y - 40) / 420)
    if y < 310:
        return 42 - 12 * np.sin(np.pi * (y - 250) / 60)
    return max(30 * (1 - (y - 310) / 190) ** 0.9 + 2, 1)


ys = np.arange(0, 500)
hw = np.array([half_width(y) for y in ys])
rot = np.array([[np.cos(ANGLE), -np.sin(ANGLE)], [np.sin(ANGLE), np.cos(ANGLE)]])
center = np.array([W / 2, 60.0])


def to_img(x, y):
    return (rot @ np.array([x, y])) + center


poly = [to_img(-h, y) for y, h in zip(ys, hw)] + [to_img(h, y) for y, h in zip(ys[::-1], hw[::-1])]
poly = np.array(poly)

# Rasterize by inverse mapping every pixel into tooth space.
gy, gx = np.mgrid[0:H, 0:W]
local = (np.stack([gx.ravel(), gy.ravel()], 1) - center) @ rot  # rot^T applied
lx, ly = local[:, 0], local[:, 1]
inside = (ly >= 0) & (ly < 500)
w_at = np.interp(ly, ys, hw)
mask = (inside & (np.abs(lx) <= w_at)).reshape(H, W)

m = measure_crr(mask)
report = m.to_dict()
report.update(
    report_schema=REPORT_SCHEMA,
    image={"file_name": "synthetic.png", "width": W, "height": H},
    bbox_xyxy=[int(np.nonzero(mask)[1].min()), int(np.nonzero(mask)[0].min()), int(np.nonzero(mask)[1].max()) + 1, int(np.nonzero(mask)[0].max()) + 1],
    contour=[[int(round(x)), int(round(y))] for x, y in poly[::4]],
    provenance={"mask_source": "synthetic", "clinical_use": False},
)
out = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("synthetic_report.json")
out.write_text(json.dumps(report, ensure_ascii=False, indent=1))
print(m.status, m.metrics["ratio"], m.metrics["neck_prominence"], "->", out)
