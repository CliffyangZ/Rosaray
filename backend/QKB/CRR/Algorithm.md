# CRR — Crown/Root Ratio (mask geometry baseline)

Algorithm id: `rosaray.crr.mask-geometry` · schema `rosaray.crr.measurement/1` · code: `backend/QKB/CRR/crr/`

Estimates a crown-to-root length ratio of **one tooth** from its segmentation mask alone (no CEJ / apex landmarks). It is a geometric baseline for comparison with expert landmarks later — **not a clinical CRR**.

## 1. Pipeline

```
image ──(SAM, optional)──▶ mask ──▶ measure_crr(mask, params) ──▶ CrrMeasurement ──▶ report JSON ──▶ frontend viewer
              COCO polygon/RLE ─┘                                       (build_report adds contour + provenance)
```

`measure_crr` steps (ported from `ratio_algo.ipynb`):

1. **Long axis** — PCA over all foreground pixels; the eigenvector of the largest eigenvalue is the axis (`t`), its normal is `s`.
2. **Width profile** — split `[t_min, t_max]` into `n_bins` slices; width of a slice = `max(s) − min(s)` of its pixels. Smooth with a moving average (`smooth_window`, zero-padded `same` convolution).
3. **Neck (≈CEJ)** — candidates are interior bins (excluding `edge_buffer_bins` at each end) strictly narrower than both neighbours. Prominence = `(min(max width left, max width right) − width) / min(...)`. Candidates below `min_prominence_frac` are dropped; the deepest remaining one is the neck. A monotonically tapering tooth has no candidate → `neck_not_found` (no ratio is forced).
4. **Crown side** — the end whose mean width over `edge_buffer_bins` is smaller is the root apex; the other end is the crown tip.
5. **Lengths** — along the axis: `crown = |t_neck − t_crown_tip|`, `root = |t_neck − t_root_apex|`, `ratio = crown / root`.

## 2. Input

### 2.1 Python API

```python
from crr import measure_crr, CrrParams
m = measure_crr(mask, CrrParams(...))   # -> CrrMeasurement;  m.to_dict() is the JSON below
```

| Name | Type | Required | Description |
|---|---|---|---|
| `mask` | `ndarray` 2-D, bool-castable (H×W) | yes | Foreground = the one target tooth. Multi-tooth / noisy masks should be cleaned first (`crr.masks.postprocess_mask` keeps the largest component + 5×5 closing). Non-2-D raises `ValueError`. |
| `params` | `CrrParams` | no | See below. Invalid values raise `ValueError`. |

`CrrParams`:

| Field | Default | Constraint | Meaning |
|---|---|---|---|
| `n_bins` | 100 | ≥ 10 | Slices along the axis. |
| `smooth_window` | 5 | ≥ 1 | Moving-average window over widths. |
| `edge_buffer_bins` | 3 | ≥ 1, < n_bins/2 | Bins excluded at each end from neck search; also used to decide the root end. |
| `min_prominence_frac` | 0.04 | [0, 1) | Minimum relative neck depth. |
| `min_mask_pixels` | 100 | ≥ 3 | Smaller masks → `invalid_mask`. |
| `pixel_spacing_mm` | `None` | > 0 | If set, `*_mm` metrics are added. |

### 2.2 Mask sources (`crr.masks`, `crr.sam`)

| Source | Entry point | Notes |
|---|---|---|
| COCO json | `load_coco_mask(coco_path, file_name)` | Largest annotation; polygon and uncompressed RLE supported. |
| Binary PNG | `load_mask_png(path)` | Pixel > 127 = foreground. |
| SAM | `SamSegmenter(checkpoint=None, model_type="vit_b", device="auto").segment(rgb)` | Box prompt (fractions `0.32, 0.05, 0.68, 0.95` of the image) + one positive / one negative point; best of 3 masks by score; post-processed. Returns `Segmentation(mask, sam_score, box_xyxy, model, device)`. |

**Model files** live in `<Rosaray>/model/` (env `ROSARAY_MODEL_DIR` overrides). Default: `model/sam/sam_vit_b_01ec64.pth`, fetched by `python -m crr.download_model` (SHA-256 checked). `model/*` is git-ignored. `measure_crr` itself needs no model. Requirements: `requirements.txt` (`torch` + `segment-anything` only for SAM).

### 2.3 CLI

```
python -m crr measure --image 1.png --coco coco_annotations.json --out 1.crr.json
python -m crr measure --image 1.png --mask mask.png --out 1.crr.json
python -m crr measure --image 1.png --out 1.crr.json      # SAM
   options: --n-bins --smooth-window --min-prominence --pixel-spacing-mm --checkpoint --device
```

## 3. Output — `CrrMeasurement.to_dict()`

All coordinates are image pixels `[x, y]` (origin top-left, y down). All lengths are mask pixels unless suffixed `_mm`. Numbers are finite or `null` (never NaN); the JSON is strict.

| Field | Type | Description |
|---|---|---|
| `schema` | string | `rosaray.crr.measurement/1` |
| `status` | string | `ok` · `neck_not_found` · `invalid_mask` |
| `found` | bool | `status == "ok"` |
| `message` | string | Reason when not `ok`, else `""`. |
| `metrics` | object | §3.1 |
| `geometry` | object | §3.2 (empty `{}` for `invalid_mask`) |
| `profile` | object | §3.3 (empty `{}` for `invalid_mask`) |
| `params` | object | The `CrrParams` used. |

### 3.1 `metrics`

| Key | Unit | When | Meaning |
|---|---|---|---|
| `mask_area_px` | px² | always | Foreground pixel count. |
| `ratio` | — | `ok` | `crown_length_px / root_length_px`; `null` otherwise. |
| `crown_length_px`, `root_length_px` | px | `ok` | Along the long axis, split at the neck. |
| `crown_fraction` | 0–1 | `ok` | `crown / (crown + root)`. |
| `tooth_length_px` | px | not invalid | Extent along the axis. |
| `max_width_px` | px | not invalid | Max of smoothed width profile. |
| `neck_width_px` | px | `ok` | Raw width at the neck bin. |
| `neck_width_rel` | 0–1 | `ok` | `neck_width_px / max_width_px`. |
| `neck_prominence` | 0–1 | `ok` | Relative depth of the chosen neck (a confidence cue). |
| `n_neck_candidates` | int | not invalid | Local minima found before the prominence filter. |
| `axis_angle_deg` | ° [0,180) | not invalid | Axis angle vs +x in image coordinates. |
| `centroid` | `[x,y]` | not invalid | Mask centroid. |
| `pixel_spacing_mm`, `crown_length_mm`, `root_length_mm`, `tooth_length_mm`, `max_width_mm`, `neck_width_mm` | mm | `ok` and spacing set | px × `pixel_spacing_mm`. |

Keys that do not apply hold `null`.

### 3.2 `geometry`

| Key | Meaning |
|---|---|
| `axis_origin`, `axis_direction` | Centroid and unit axis vector. |
| `axis_extent` | `[start, end]` axis endpoints (`t_min`, `t_max`). |
| `crown_tip`, `root_apex`, `neck` | Points on the axis (`null` unless `ok`). |
| `neck_line` | `[p1, p2]` — mask extent perpendicular to the axis at the neck. |

### 3.3 `profile` (for the width chart)

`t` (bin centre along axis, px from centroid), `width` (raw), `width_smooth`: equal-length arrays of `n_bins`; `neck_index` and `crown_end_index` (`0` or `n_bins−1`: which array end is the crown) — both `null` unless `ok`.

### 3.4 Failure semantics

| Status | Cause | `ratio` |
|---|---|---|
| `invalid_mask` | mask area < `min_mask_pixels` | `null` |
| `neck_not_found` | no local minimum with prominence ≥ `min_prominence_frac` (or root length 0) | `null` |

`measure_crr` does not raise for bad masks; callers must check `status`.

## 4. Determinism

Pure function of `(mask, params)`; no randomness. SAM output depends on weights, device and torch version, so the report records `mask_source`, SAM score and box.

## 5. Viewer report — `rosaray.crr.report/1`

`crr.report.build_report(measurement, mask, image_name, mask_source, extra)` = `to_dict()` plus:

| Field | Description |
|---|---|
| `report_schema` | `rosaray.crr.report/1` |
| `image` | `{file_name, width, height}` — the SVG coordinate space. |
| `bbox_xyxy` | `[x1,y1,x2,y2]` tight box of the mask (x2/y2 exclusive). Older reports may lack it; the viewer then derives it from `contour`. |
| `contour` | `[[x,y],…]` outer contour of the largest region (OpenCV, ε = 1 px) = the segmentation drawn by the viewer. |
| `provenance` | `{mask_source, clinical_use: false, sam_score?, box_xyxy?}` |

Frontend (`frontend/src/crr_view.js`, tab **CRR 量測**): read-only. Loads one or more report JSON files (file picker / drop; array allowed), optionally matching image files by `image.file_name`, or a bundled sample. It draws contour, axis, crown (blue) / root (green) segments, neck line and point, the metric cards, and the width-profile chart with the neck and crown/root zones. It computes nothing and stores nothing.

### Knowledge base

`crr/manifest.py` describes this algorithm (method, inputs, params, output metrics, status codes, limits) as JSON; regenerate with `python -m crr.manifest --out manifest.json --out ../../../frontend/src/crr_manifest.json`. The frontend Knowledge base dialog (⌘K / Ctrl+K, or the button on the Evidence page) renders it read-only. `tests/test_manifest.py` fails if the manifest, the committed copies, or the real metric/geometry keys drift apart. Add a new algorithm by appending to `build()["algorithms"]`.

## 6. Limitations

- Geometric approximation only; multi-root teeth, occlusion and adjacent-tooth merging can misplace the neck.
- SAM masks are pseudo-labels until expert-corrected; mask error passes straight into the ratio.
- Output is for observing algorithm behaviour and picking cases to refine, not a medical conclusion.
- Pixel units unless `pixel_spacing_mm` is provided (projection radiographs are not calibrated by default).

## 7. Tests / demo

```
cd backend/QKB/CRR
python -m unittest discover -s tests                            # numpy only
python examples/make_synthetic_report.py examples/synthetic_report.json   # demo report (also frontend/src/crr_sample.json)
```
