"""Machine-readable description of the QKB Python measurement algorithms.

The frontend Knowledge base renders this file (read-only). Regenerate with

    python -m crr.manifest --out manifest.json --out ../../../frontend/src/crr_manifest.json

``tests/test_manifest.py`` fails if the committed copies drift from this module.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import asdict
from pathlib import Path
from typing import Any, Dict, List

from .geometry import SCHEMA, STATUS_INVALID_MASK, STATUS_NO_NECK, STATUS_OK, CrrParams

MANIFEST_SCHEMA = "rosaray.qkb.algorithms/1"

_PARAM_DOC = {
    "n_bins": ("沿長軸切分的段數", "≥ 10"),
    "smooth_window": ("寬度輪廓的移動平均視窗", "≥ 1"),
    "edge_buffer_bins": ("頸部搜尋時兩端排除的段數；也用來判斷哪一端是根尖", "≥ 1 且 < n_bins / 2"),
    "min_prominence_frac": ("頸部凹陷的最小相對深度，低於此值視為找不到頸部", "[0, 1)"),
    "min_mask_pixels": ("mask 最少像素數，少於此值回報 invalid_mask", "≥ 3"),
    "pixel_spacing_mm": ("像素間距（mm）；提供後才會輸出 *_mm 指標", "> 0，可為 null"),
}

# (key, 單位, 說明, 何時有值)
_METRICS = [
    ("ratio", "—", "牙冠長度 ÷ 牙根長度", "status = ok"),
    ("crown_length_px", "px", "沿長軸，牙冠端到頸部的長度", "status = ok"),
    ("root_length_px", "px", "沿長軸，頸部到根尖的長度", "status = ok"),
    ("crown_fraction", "0–1", "牙冠 ÷ (牙冠 + 牙根)", "status = ok"),
    ("neck_width_px", "px", "頸部處垂直於長軸的寬度", "status = ok"),
    ("neck_width_rel", "0–1", "頸部寬度 ÷ 最大寬度", "status = ok"),
    ("neck_prominence", "0–1", "頸部凹陷的相對深度（信心參考）", "status = ok"),
    ("tooth_length_px", "px", "牙齒沿長軸的全長", "mask 有效"),
    ("max_width_px", "px", "平滑後寬度輪廓的最大值", "mask 有效"),
    ("n_neck_candidates", "個", "通過前的局部最小值個數", "mask 有效"),
    ("axis_angle_deg", "°", "長軸與 +x 的夾角（影像座標，0–180）", "mask 有效"),
    ("centroid", "px [x,y]", "mask 重心", "mask 有效"),
    ("mask_area_px", "px²", "前景像素數", "一律"),
    ("pixel_spacing_mm", "mm/px", "使用者提供的像素間距", "提供 pixel_spacing_mm 時"),
    ("crown_length_mm", "mm", "牙冠長度（mm）", "提供 pixel_spacing_mm 時"),
    ("root_length_mm", "mm", "牙根長度（mm）", "提供 pixel_spacing_mm 時"),
    ("tooth_length_mm", "mm", "牙齒全長（mm）", "提供 pixel_spacing_mm 時"),
    ("max_width_mm", "mm", "最大寬度（mm）", "提供 pixel_spacing_mm 時"),
    ("neck_width_mm", "mm", "頸部寬度（mm）", "提供 pixel_spacing_mm 時"),
]


def build() -> Dict[str, Any]:
    defaults = asdict(CrrParams())
    params: List[Dict[str, Any]] = [
        {"name": k, "default": defaults[k], "constraint": _PARAM_DOC[k][1], "description": _PARAM_DOC[k][0]}
        for k in defaults
    ]
    crr = {
        "id": "rosaray.crr.mask-geometry",
        "name": "CRR 冠根比（mask 幾何）",
        "version": "0.1.0",
        "language": "python",
        "package": "backend/QKB/CRR/crr",
        "evidence_type": "crr",
        "summary": "僅由單顆牙齒的 segmentation mask，推算牙冠與牙根沿長軸的長度與冠根比。",
        "intended_use": "作為與專家 CEJ／根尖標註比較前的純幾何基準線，觀察演算法行為並挑出需要優化的案例。",
        "method": [
            "PCA：對 mask 前景像素取最大變異方向為牙齒長軸。",
            "寬度輪廓：沿長軸切成 n_bins 段，量測每段垂直於長軸的寬度，並做移動平均。",
            "頸部：在寬度輪廓上找左右都比它寬的局部最小值，以相對凹陷深度過濾；取最深者為頸部（≈CEJ）。單調收尖的形狀不會被誤判，找不到時不硬給比例。",
            "冠／根方向：兩端平均寬度較小的一端為根尖，另一端為牙冠端。",
            "長度與比例：沿長軸計算頸部到兩端的距離，ratio = 牙冠長度 ÷ 牙根長度。",
        ],
        "inputs": [
            {"name": "mask", "type": "2-D 陣列（H×W，可轉 bool）", "required": True,
             "description": "前景為單一目標牙齒。多牙或雜訊請先清理（crr.masks.postprocess_mask：最大連通區 + 5×5 closing）。"},
            {"name": "params", "type": "CrrParams", "required": False, "description": "見下方參數表。"},
        ],
        "mask_sources": [
            {"id": "coco", "label": "COCO json", "entry": "load_coco_mask(coco_path, file_name)", "note": "polygon 與 uncompressed RLE"},
            {"id": "png", "label": "二值 mask PNG", "entry": "load_mask_png(path)", "note": "像素 > 127 為前景"},
            {"id": "sam", "label": "SAM ViT-B", "entry": "SamSegmenter().segment(rgb)", "note": "權重放在 Rosaray/model/sam/；需 torch 與 segment-anything"},
        ],
        "params": params,
        "outputs": {
            "schema": SCHEMA,
            "metrics": [{"key": k, "unit": u, "description": d, "when": w} for k, u, d, w in _METRICS],
            "geometry": ["axis_origin", "axis_direction", "axis_extent", "crown_tip", "root_apex", "neck", "neck_line"],
            "profile": ["t", "width", "width_smooth", "neck_index", "crown_end_index"],
            "report_extras": ["report_schema", "image", "bbox_xyxy", "contour", "provenance"],
        },
        "statuses": [
            {"code": STATUS_OK, "description": "找到頸部並回報 ratio。"},
            {"code": STATUS_NO_NECK, "description": "沒有局部最小值達到 min_prominence_frac（或牙根長度為 0）；不回報 ratio。"},
            {"code": STATUS_INVALID_MASK, "description": "mask 面積小於 min_mask_pixels。"},
        ],
        "evidence_layers": ["Segmentation", "Bounding box", "長軸", "冠 / 根 / 頸部"],
        "usage": {
            "python": "from crr import measure_crr, CrrParams\nm = measure_crr(mask, CrrParams())\nm.to_dict()",
            "cli": "python -m crr measure --image 1.png --coco coco_annotations.json --out 1.crr.json",
        },
        "limitations": [
            "純幾何近似，不是臨床 CRR。",
            "多根牙、影像遮蔽、鄰牙沾黏會讓頸部落錯位置。",
            "SAM mask 為 pseudo-label，邊界誤差會直接進入 ratio；真實影像上常把鄰近骨組織一併框入。",
            "預設單位為像素；未提供 pixel_spacing_mm 時不換算 mm。",
            "只供觀察演算法行為與挑選待優化案例，不可作為醫學結論。",
        ],
        "spec": "backend/QKB/CRR/Algorithm.md",
    }
    return {"schema": MANIFEST_SCHEMA, "algorithms": [crr]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, action="append", help="write here (repeatable); default stdout")
    args = parser.parse_args()
    text = json.dumps(build(), ensure_ascii=False, indent=2) + "\n"
    for out in args.out or []:
        out.write_text(text, encoding="utf-8")
    if not args.out:
        print(text, end="")


if __name__ == "__main__":
    main()
