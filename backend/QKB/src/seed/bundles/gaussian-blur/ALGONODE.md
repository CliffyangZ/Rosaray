---
schema: quantify-kb/1
kind: algonode
id: rosaray.gaussian-blur
version: 1.0.0
name: Gaussian blur
summary: Smooths an image with a Gaussian-like blur.
domain: dental-image
status: draft
research_use_only: true
intended_use: Suppress noise before segmentation.
limitations: Approximated by two box blurs; sigma is in pixels, not millimetres.
---
Two successive box blurs of radius round(0.9 x sigma).
