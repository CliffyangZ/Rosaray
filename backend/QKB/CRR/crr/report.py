"""Viewer report: a measurement plus what the frontend needs to draw it
(image size, tooth contour, provenance). Schema: Algorithm.md section 5."""

from __future__ import annotations

from typing import Any, Dict, Optional, Tuple

import numpy as np

from .geometry import CrrMeasurement
REPORT_SCHEMA = "rosaray.crr.report/1"


def build_report(
    measurement: CrrMeasurement,
    mask: np.ndarray,
    image_name: Optional[str] = None,
    mask_source: str = "unknown",
    extra: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    from .masks import largest_contour  # needs OpenCV

    h, w = mask.shape[:2]
    ys, xs = np.nonzero(mask)
    bbox = [int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1] if xs.size else None
    report = measurement.to_dict()
    report.update(
        report_schema=REPORT_SCHEMA,
        image={"file_name": image_name, "width": int(w), "height": int(h)},
        bbox_xyxy=bbox,
        contour=largest_contour(mask),
        provenance={"mask_source": mask_source, "clinical_use": False, **(extra or {})},
    )
    return report
