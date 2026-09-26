---
schema: quantify-kb/1
kind: algonode
id: rosaray.area
version: 1.0.0
name: Area measurement
summary: Measures the area and component count of a mask.
domain: dental-image
status: draft
research_use_only: true
intended_use: Quantify a segmented region in millimetres squared for research reporting.
limitations: Requires the image to carry pixel spacing; counts pixels of the mask, not clinical area.
---
Columns: pixels (count), mm2 (pixels x spacing x spacing), components (4-connected).
