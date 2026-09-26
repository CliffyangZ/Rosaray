"""Mask-geometry Crown/Root Ratio (CRR) measurement. NumPy only.

Ported from ``ratio_algo.ipynb``: PCA long axis -> width profile along the axis
-> deepest local width minimum (neck, ~CEJ) -> crown/root lengths and ratio.
All lengths are in pixels of the input mask. This is a geometric approximation,
not a clinical CRR (see Algorithm.md, "Limitations").
"""

from __future__ import annotations

from dataclasses import asdict, dataclass
from typing import Any, Dict, List, Optional

import numpy as np

SCHEMA = "rosaray.crr.measurement/1"

STATUS_OK = "ok"
STATUS_NO_NECK = "neck_not_found"
STATUS_INVALID_MASK = "invalid_mask"


@dataclass(frozen=True)
class CrrParams:
    n_bins: int = 100
    smooth_window: int = 5
    edge_buffer_bins: int = 3
    min_prominence_frac: float = 0.04
    min_mask_pixels: int = 100
    pixel_spacing_mm: Optional[float] = None

    def validate(self) -> None:
        if self.n_bins < 10:
            raise ValueError("n_bins must be >= 10")
        if self.smooth_window < 1:
            raise ValueError("smooth_window must be >= 1")
        if self.edge_buffer_bins < 1 or 2 * self.edge_buffer_bins >= self.n_bins:
            raise ValueError("edge_buffer_bins must be >= 1 and < n_bins / 2")
        if not 0 <= self.min_prominence_frac < 1:
            raise ValueError("min_prominence_frac must be in [0, 1)")
        if self.min_mask_pixels < 3:
            raise ValueError("min_mask_pixels must be >= 3")
        if self.pixel_spacing_mm is not None and self.pixel_spacing_mm <= 0:
            raise ValueError("pixel_spacing_mm must be > 0")


@dataclass
class CrrMeasurement:
    """Result of :func:`measure_crr`. ``to_dict()`` is the JSON contract."""

    status: str
    params: CrrParams
    metrics: Dict[str, Any]
    geometry: Dict[str, Any]
    profile: Dict[str, Any]
    message: str = ""

    @property
    def found(self) -> bool:
        return self.status == STATUS_OK

    @property
    def ratio(self) -> Optional[float]:
        return self.metrics.get("ratio")

    def to_dict(self) -> Dict[str, Any]:
        return {
            "schema": SCHEMA,
            "status": self.status,
            "found": self.found,
            "message": self.message,
            "metrics": self.metrics,
            "geometry": self.geometry,
            "profile": self.profile,
            "params": asdict(self.params),
        }


def _pt(p: np.ndarray) -> List[float]:
    return [round(float(p[0]), 2), round(float(p[1]), 2)]


def _f(v: float, nd: int = 4) -> float:
    return round(float(v), nd)


def compute_axis_profile(mask: np.ndarray, n_bins: int = 100, smooth_window: int = 5) -> Dict[str, Any]:
    """PCA long axis, then the perpendicular extent of the mask in ``n_bins`` slices."""
    ys, xs = np.nonzero(mask)
    points = np.stack([xs, ys], axis=1).astype(np.float64)

    centroid = points.mean(axis=0)
    centered = points - centroid
    eigvals, eigvecs = np.linalg.eigh(np.cov(centered.T))
    axis = eigvecs[:, np.argmax(eigvals)]
    axis = axis / np.linalg.norm(axis)
    perp = np.array([-axis[1], axis[0]])

    t = centered @ axis
    s = centered @ perp

    t_min, t_max = float(t.min()), float(t.max())
    edges = np.linspace(t_min, t_max, n_bins + 1)
    centers = (edges[:-1] + edges[1:]) / 2
    bin_idx = np.clip(np.digitize(t, edges) - 1, 0, n_bins - 1)

    s_min = np.zeros(n_bins)
    s_max = np.zeros(n_bins)
    widths = np.zeros(n_bins)
    for i in range(n_bins):
        sel = s[bin_idx == i]
        if sel.size:
            s_min[i], s_max[i] = sel.min(), sel.max()
            widths[i] = s_max[i] - s_min[i]

    if smooth_window > 1:
        widths_smooth = np.convolve(widths, np.ones(smooth_window) / smooth_window, mode="same")
    else:
        widths_smooth = widths.copy()

    return {
        "centroid": centroid,
        "axis": axis,
        "perp": perp,
        "t_min": t_min,
        "t_max": t_max,
        "bin_centers": centers,
        "s_min": s_min,
        "s_max": s_max,
        "widths": widths,
        "widths_smooth": widths_smooth,
    }


def find_neck(profile: Dict[str, Any], edge_buffer_bins: int = 3, min_prominence_frac: float = 0.04) -> Dict[str, Any]:
    """Find the neck (~CEJ): the local width minimum with the largest relative depth.

    Prominence = (lower of the two flanking width maxima - width at neck) / that
    lower maximum. A monotonically tapering root has no interior local minimum,
    so it is never reported as a neck; if no candidate reaches
    ``min_prominence_frac`` the result is ``found=False`` (no forced ratio).
    """
    widths = profile["widths_smooth"]
    centers = profile["bin_centers"]
    n = len(widths)

    candidates = []
    for i in range(edge_buffer_bins, n - edge_buffer_bins):
        if widths[i] <= 0:
            continue
        if widths[i] < widths[i - 1] and widths[i] < widths[i + 1]:
            ridge = min(widths[:i].max(), widths[i:].max())
            if ridge <= 0:
                continue
            candidates.append(((ridge - widths[i]) / ridge, i))

    valid = [c for c in candidates if c[0] >= min_prominence_frac]
    out: Dict[str, Any] = {"found": False, "n_candidates": len(candidates), "n_valid": len(valid)}
    if not valid:
        return out

    prominence, idx = max(valid, key=lambda c: c[0])
    t_neck = float(centers[idx])

    head, tail = widths[:edge_buffer_bins], widths[-edge_buffer_bins:]
    w_head = head[head > 0].mean() if np.any(head > 0) else 0.0
    w_tail = tail[tail > 0].mean() if np.any(tail > 0) else 0.0
    # The narrower end is the root apex side.
    if w_head <= w_tail:
        root_end_t, crown_end_t = profile["t_min"], profile["t_max"]
    else:
        root_end_t, crown_end_t = profile["t_max"], profile["t_min"]

    root_length = abs(t_neck - root_end_t)
    crown_length = abs(t_neck - crown_end_t)
    out.update(
        found=True,
        index=int(idx),
        prominence=float(prominence),
        t_neck=t_neck,
        root_end_t=root_end_t,
        crown_end_t=crown_end_t,
        root_length=root_length,
        crown_length=crown_length,
    )
    return out


def measure_crr(mask: np.ndarray, params: Optional[CrrParams] = None) -> CrrMeasurement:
    """Measure one tooth from a boolean mask (H x W). Never raises on bad masks;
    check ``status`` instead. Invalid ``params`` raise ``ValueError``."""
    params = params or CrrParams()
    params.validate()

    mask = np.asarray(mask)
    if mask.ndim != 2:
        raise ValueError(f"mask must be 2-D, got shape {mask.shape}")
    mask = mask.astype(bool)
    area = int(mask.sum())
    if area < params.min_mask_pixels:
        return CrrMeasurement(
            status=STATUS_INVALID_MASK,
            params=params,
            metrics={"mask_area_px": area, "ratio": None},
            geometry={},
            profile={},
            message=f"mask has {area} px (< min_mask_pixels={params.min_mask_pixels})",
        )

    prof = compute_axis_profile(mask, params.n_bins, params.smooth_window)
    neck = find_neck(prof, params.edge_buffer_bins, params.min_prominence_frac)
    c, ax, pp = prof["centroid"], prof["axis"], prof["perp"]
    at = lambda t: c + ax * t  # noqa: E731

    tooth_len = prof["t_max"] - prof["t_min"]
    max_width = float(prof["widths_smooth"].max())
    spacing = params.pixel_spacing_mm

    metrics: Dict[str, Any] = {
        "mask_area_px": area,
        "tooth_length_px": _f(tooth_len, 2),
        "max_width_px": _f(max_width, 2),
        "axis_angle_deg": _f(np.degrees(np.arctan2(ax[1], ax[0])) % 180.0, 2),
        "centroid": _pt(c),
        "crown_length_px": None,
        "root_length_px": None,
        "ratio": None,
        "crown_fraction": None,
        "neck_width_px": None,
        "neck_width_rel": None,
        "neck_prominence": None,
        "n_neck_candidates": neck["n_candidates"],
    }
    geometry: Dict[str, Any] = {
        "axis_origin": _pt(c),
        "axis_direction": [_f(ax[0], 6), _f(ax[1], 6)],
        "axis_extent": [_pt(at(prof["t_min"])), _pt(at(prof["t_max"]))],
        "crown_tip": None,
        "root_apex": None,
        "neck": None,
        "neck_line": None,
    }
    profile = {
        "t": [_f(v, 2) for v in prof["bin_centers"]],
        "width": [_f(v, 2) for v in prof["widths"]],
        "width_smooth": [_f(v, 2) for v in prof["widths_smooth"]],
        "neck_index": None,
        "crown_end_index": None,
    }

    if not neck["found"]:
        return CrrMeasurement(
            status=STATUS_NO_NECK,
            params=params,
            metrics=metrics,
            geometry=geometry,
            profile=profile,
            message="no width-profile local minimum reached min_prominence_frac; ratio not reported",
        )

    i = neck["index"]
    crown, root = neck["crown_length"], neck["root_length"]
    ratio = crown / root if root > 0 else None
    neck_width = float(prof["widths"][i])
    metrics.update(
        crown_length_px=_f(crown, 2),
        root_length_px=_f(root, 2),
        ratio=None if ratio is None else _f(ratio),
        crown_fraction=_f(crown / (crown + root)) if crown + root > 0 else None,
        neck_width_px=_f(neck_width, 2),
        neck_width_rel=_f(neck_width / max_width) if max_width > 0 else None,
        neck_prominence=_f(neck["prominence"]),
    )
    if spacing is not None:
        metrics.update(
            pixel_spacing_mm=spacing,
            crown_length_mm=_f(crown * spacing, 3),
            root_length_mm=_f(root * spacing, 3),
            tooth_length_mm=_f(tooth_len * spacing, 3),
            max_width_mm=_f(max_width * spacing, 3),
            neck_width_mm=_f(neck_width * spacing, 3),
        )
    neck_c = at(neck["t_neck"])
    geometry.update(
        crown_tip=_pt(at(neck["crown_end_t"])),
        root_apex=_pt(at(neck["root_end_t"])),
        neck=_pt(neck_c),
        neck_line=[
            _pt(neck_c + pp * prof["s_min"][i]),
            _pt(neck_c + pp * prof["s_max"][i]),
        ],
    )
    profile["neck_index"] = i
    profile["crown_end_index"] = 0 if neck["crown_end_t"] == prof["t_min"] else len(prof["bin_centers"]) - 1
    return CrrMeasurement(
        status=STATUS_OK if ratio is not None else STATUS_NO_NECK,
        params=params,
        metrics=metrics,
        geometry=geometry,
        profile=profile,
        message="" if ratio is not None else "root length is zero",
    )
