---
schema: quantify-kb/1
kind: algonode
id: rosaray.morphology
version: 1.0.0
name: Morphology
summary: Opens, closes, erodes or dilates a binary mask.
domain: dental-image
status: draft
research_use_only: true
intended_use: Remove specks or fill small gaps in a segmentation mask.
limitations: Square structuring element; radius is in pixels.
---
Standard binary morphology with a square structuring element.
