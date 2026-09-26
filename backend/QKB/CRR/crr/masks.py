"""Mask I/O helpers (need OpenCV): COCO polygon / uncompressed-RLE decoding,
mask cleanup and contour extraction."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

import cv2
import numpy as np


def decode_uncompressed_rle(rle: Dict[str, Any], height: int, width: int) -> np.ndarray:
    """COCO uncompressed RLE: column-major runs, background first."""
    rle_h, rle_w = rle["size"]
    flat = np.zeros(rle_h * rle_w, dtype=np.uint8)
    pos, value = 0, 0
    for run in rle["counts"]:
        if value:
            flat[pos : pos + run] = 1
        pos += run
        value = 1 - value
    mask = flat.reshape((rle_h, rle_w), order="F")
    if (rle_h, rle_w) != (height, width):
        mask = cv2.resize(mask, (width, height), interpolation=cv2.INTER_NEAREST)
    return mask.astype(bool)


def segmentation_to_mask(segmentation: Any, height: int, width: int) -> np.ndarray:
    if isinstance(segmentation, dict):
        return decode_uncompressed_rle(segmentation, height, width)
    mask = np.zeros((height, width), dtype=np.uint8)
    for polygon in segmentation:
        pts = np.array(polygon, dtype=np.float64).reshape(-1, 2)
        cv2.fillPoly(mask, [pts.round().astype(np.int32)], color=1)
    return mask.astype(bool)


def load_coco_mask(coco_path: Path, file_name: str) -> Tuple[np.ndarray, Tuple[int, int]]:
    """Largest annotation of ``file_name`` as a mask; returns (mask, (height, width))."""
    coco = json.loads(Path(coco_path).read_text(encoding="utf-8"))
    record = next((im for im in coco["images"] if im["file_name"] == file_name), None)
    if record is None:
        raise FileNotFoundError(f"{file_name} not found in {coco_path}")
    anns = [a for a in coco["annotations"] if a["image_id"] == record["id"]]
    if not anns:
        raise LookupError(f"{file_name} has no annotation in {coco_path}")
    ann = max(anns, key=lambda a: a["area"])
    h, w = record["height"], record["width"]
    return segmentation_to_mask(ann["segmentation"], h, w), (h, w)


def load_mask_png(path: Path) -> np.ndarray:
    img = cv2.imread(str(path), cv2.IMREAD_GRAYSCALE)
    if img is None:
        raise FileNotFoundError(f"cannot read mask: {path}")
    return img > 127


def keep_largest_component(mask: np.ndarray) -> np.ndarray:
    n, labels, stats, _ = cv2.connectedComponentsWithStats(mask.astype(np.uint8), connectivity=8)
    if n <= 1:
        return mask.astype(bool)
    return labels == 1 + int(np.argmax(stats[1:, cv2.CC_STAT_AREA]))


def postprocess_mask(mask: np.ndarray) -> np.ndarray:
    """Largest connected component + 5x5 closing (same as the SAM pseudo-label pipeline)."""
    clean = keep_largest_component(mask).astype(np.uint8)
    clean = cv2.morphologyEx(clean, cv2.MORPH_CLOSE, np.ones((5, 5), np.uint8))
    return clean.astype(bool)


def largest_contour(mask: np.ndarray, epsilon: float = 1.0) -> List[List[float]]:
    """Outer contour of the largest region as [[x, y], ...] (simplified)."""
    contours, _ = cv2.findContours(mask.astype(np.uint8), cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_NONE)
    if not contours:
        return []
    best = max(contours, key=cv2.contourArea)
    if epsilon > 0:
        best = cv2.approxPolyDP(best, epsilon, True)
    return [[int(x), int(y)] for x, y in best.reshape(-1, 2)]
