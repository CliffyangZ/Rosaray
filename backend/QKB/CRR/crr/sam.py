"""SAM (ViT) tooth segmentation that yields the mask fed to ``measure_crr``.

Weights live in ``<Rosaray>/model/`` (override with ``ROSARAY_MODEL_DIR``); see
``python -m crr.download_model``. Torch / segment_anything are imported lazily
so the rest of the package works without them.

Prompting follows the SAM pseudo-label pipeline: a box over the central tooth
plus a positive point at its center and a negative point near its inner edge.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional, Tuple

import numpy as np

from .masks import postprocess_mask

BOX_FRACTIONS = (0.32, 0.05, 0.68, 0.95)
DEFAULT_MODEL_TYPE = "vit_b"
DEFAULT_CHECKPOINT_NAME = "sam_vit_b_01ec64.pth"


def model_dir() -> Path:
    env = os.environ.get("ROSARAY_MODEL_DIR")
    if env:
        return Path(env).expanduser()
    return Path(__file__).resolve().parents[4] / "model"


def default_checkpoint() -> Path:
    return model_dir() / "sam" / DEFAULT_CHECKPOINT_NAME


@dataclass
class Segmentation:
    mask: np.ndarray
    sam_score: float
    box_xyxy: Tuple[int, int, int, int]
    model: str
    device: str


class SamSegmenter:
    def __init__(self, checkpoint: Optional[Path] = None, model_type: str = DEFAULT_MODEL_TYPE, device: str = "auto"):
        try:
            import torch
            from segment_anything import SamPredictor, sam_model_registry
        except ImportError as exc:  # pragma: no cover - environment dependent
            raise RuntimeError("SAM needs torch and segment-anything; see requirements.txt") from exc

        checkpoint = Path(checkpoint) if checkpoint else default_checkpoint()
        if not checkpoint.is_file():
            raise FileNotFoundError(
                f"SAM checkpoint not found: {checkpoint}. Run `python -m crr.download_model`."
            )
        if device == "auto":
            device = "cuda" if torch.cuda.is_available() else "cpu"
        sam = sam_model_registry[model_type](checkpoint=str(checkpoint))
        sam.to(device=device)
        sam.eval()
        self._torch: Any = torch
        self._predictor = SamPredictor(sam)
        self.model = f"sam_{model_type}"
        self.checkpoint = checkpoint.name
        self.device = device

    def segment(self, image_rgb: np.ndarray, box_fractions=BOX_FRACTIONS) -> Segmentation:
        h, w = image_rgb.shape[:2]
        x1, y1, x2, y2 = (int(box_fractions[0] * w), int(box_fractions[1] * h),
                          int(box_fractions[2] * w), int(box_fractions[3] * h))
        box = np.array([x1, y1, x2, y2], dtype=np.float32)
        coords = np.array([[(x1 + x2) / 2, (y1 + y2) / 2], [x1 + 0.03 * (x2 - x1), (y1 + y2) / 2]], dtype=np.float32)
        labels = np.array([1, 0], dtype=np.int32)
        with self._torch.inference_mode():
            self._predictor.set_image(image_rgb)
            masks, scores, _ = self._predictor.predict(
                box=box, point_coords=coords, point_labels=labels, multimask_output=True
            )
        best = int(np.argmax(scores))
        return Segmentation(
            mask=postprocess_mask(masks[best]),
            sam_score=float(scores[best]),
            box_xyxy=(x1, y1, x2, y2),
            model=self.model,
            device=self.device,
        )
