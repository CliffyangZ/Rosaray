---
schema: quantify-kb/1
kind: algonode
id: rosaray.normalize
version: 1.0.0
name: Normalize
summary: Rescales intensities between two percentile bounds.
domain: dental-image
status: draft
research_use_only: true
intended_use: Reduce brightness differences between images before thresholding.
limitations: Percentile stretch only; does not correct uneven illumination.
---
Percentile-based contrast stretch.
