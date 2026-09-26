---
schema: quantify-kb/1
kind: algonode
id: rosaray.threshold
version: 1.0.0
name: Threshold
summary: Binarizes an image by a global intensity threshold.
domain: dental-image
status: draft
research_use_only: true
intended_use: Foreground/background separation for research measurement pipelines.
limitations: Global threshold; not robust to uneven illumination.
---
Pixels above the threshold become 1 (or 0 when inverted).
