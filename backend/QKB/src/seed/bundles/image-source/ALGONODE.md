---
schema: quantify-kb/1
kind: algonode
id: rosaray.image-source
version: 1.0.0
name: Image source
summary: Provides the selected intraoral image as the pipe input.
domain: dental-image
status: draft
research_use_only: true
intended_use: Supplies the image a research pipeline analyses.
limitations: Passes pixels through unchanged; carries no calibration of its own.
---
The pipeline's single entry point. Domain pipe rules require exactly one of these per dental-image pipe.
