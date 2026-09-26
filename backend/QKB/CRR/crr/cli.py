"""Command line: measure one tooth image and write a viewer report JSON.

    python -m crr measure --image 1.png --coco coco_annotations.json --out 1.crr.json
    python -m crr measure --image 1.png --mask mask.png --out 1.crr.json
    python -m crr measure --image 1.png --out 1.crr.json          # SAM from Rosaray/model/
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import cv2

from .geometry import CrrParams, measure_crr
from .masks import load_coco_mask, load_mask_png, postprocess_mask
from .report import build_report


def _measure(args: argparse.Namespace) -> None:
    bgr = cv2.imread(str(args.image), cv2.IMREAD_COLOR)
    if bgr is None:
        raise SystemExit(f"cannot read image: {args.image}")
    extra = {}
    if args.coco:
        mask, _ = load_coco_mask(args.coco, args.image.name)
        source = "coco"
    elif args.mask:
        mask = postprocess_mask(load_mask_png(args.mask))
        source = "mask_png"
    else:
        from .sam import SamSegmenter

        seg = SamSegmenter(args.checkpoint, device=args.device).segment(cv2.cvtColor(bgr, cv2.COLOR_BGR2RGB))
        mask, source = seg.mask, seg.model
        extra = {"sam_score": round(seg.sam_score, 4), "box_xyxy": list(seg.box_xyxy)}
    if mask.shape != bgr.shape[:2]:
        raise SystemExit(f"mask shape {mask.shape} != image shape {bgr.shape[:2]}")

    params = CrrParams(n_bins=args.n_bins, smooth_window=args.smooth_window,
                       min_prominence_frac=args.min_prominence, pixel_spacing_mm=args.pixel_spacing_mm)
    report = build_report(measure_crr(mask, params), mask, args.image.name, source, extra)
    text = json.dumps(report, ensure_ascii=False, indent=2)
    if args.out:
        args.out.write_text(text, encoding="utf-8")
        m = report["metrics"]
        print(f"{report['status']}  ratio={m['ratio']}  -> {args.out}")
    else:
        print(text)


def main() -> None:
    p = argparse.ArgumentParser(prog="crr", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    m = sub.add_parser("measure", help="measure one image")
    m.add_argument("--image", type=Path, required=True)
    src = m.add_mutually_exclusive_group()
    src.add_argument("--coco", type=Path, help="COCO json holding the tooth mask")
    src.add_argument("--mask", type=Path, help="binary mask PNG")
    m.add_argument("--checkpoint", type=Path, help="SAM checkpoint (default: model/sam/...)")
    m.add_argument("--device", default="auto", choices=("auto", "cuda", "cpu"))
    m.add_argument("--n-bins", type=int, default=100)
    m.add_argument("--smooth-window", type=int, default=5)
    m.add_argument("--min-prominence", type=float, default=0.04)
    m.add_argument("--pixel-spacing-mm", type=float)
    m.add_argument("--out", type=Path)
    m.set_defaults(fn=_measure)
    args = p.parse_args()
    args.fn(args)
