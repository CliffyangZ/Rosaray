#!/usr/bin/env python3
"""Regenerates text-layer.pdf and scanned.pdf (no third-party dependencies).

text-layer.pdf: a fabricated two-page "paper" with the method statements the
extraction tests rely on (eligibility, preprocessing, segmentation, measurement,
a formula, validation, a missing unit, a passage repeated on two pages, and one
unsafe clinical claim). scanned.pdf: pages that are only an image — no text layer.
"""
import zlib


def build(objects):
    out = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = []
    for i, body in enumerate(objects, start=1):
        offsets.append(len(out))
        out += f"{i} 0 obj\n".encode() + body + b"\nendobj\n"
    xref = len(out)
    out += f"xref\n0 {len(objects) + 1}\n0000000000 65535 f \n".encode()
    for off in offsets:
        out += f"{off:010d} 00000 n \n".encode()
    out += f"trailer\n<< /Size {len(objects) + 1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    return bytes(out)


def esc(s):
    return s.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")


def text_pdf(pages):
    # 1 catalog, 2 pages, 3 font, then (page, content) pairs.
    kids = " ".join(f"{4 + 2 * i} 0 R" for i in range(len(pages)))
    objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        f"<< /Type /Pages /Kids [{kids}] /Count {len(pages)} >>".encode(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ]
    for i, lines in enumerate(pages):
        content = ["BT", "/F1 10 Tf", "13 TL", "50 780 Td"]
        for line in lines:
            content.append(f"({esc(line)}) Tj T*")
        content.append("ET")
        stream = "\n".join(content).encode("latin-1")
        page_no, content_no = 4 + 2 * i, 5 + 2 * i
        objs.append(
            f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents {content_no} 0 R /Resources << /Font << /F1 3 0 R >> >> >>".encode()
        )
        objs.append(f"<< /Length {len(stream)} >>\nstream\n".encode() + stream + b"\nendstream")
    return build(objs)


def scanned_pdf(n_pages):
    raw = zlib.compress(bytes((x * 60) % 256 for x in range(16)))
    kids = " ".join(f"{5 + 3 * i} 0 R" for i in range(n_pages))
    objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        f"<< /Type /Pages /Kids [{kids}] /Count {n_pages} >>".encode(),
        f"<< /Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /FlateDecode /Length {len(raw)} >>\nstream\n".encode() + raw + b"\nendstream",
        b"<< >>",
    ]
    for i in range(n_pages):
        page_no = 5 + 3 * i
        content = b"q 500 0 0 700 40 60 cm /Im1 Do Q"
        objs.append(f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Contents {page_no + 1} 0 R /Resources << /XObject << /Im1 3 0 R >> >> >>".encode())
        objs.append(f"<< /Length {len(content)} >>\nstream\n".encode() + content + b"\nendstream")
        objs.append(b"<< >>")  # spacer keeps object numbering regular
    return build(objs)


PAGE1 = [
    "Automated Gingival Area Measurement from Intraoral Photographs",
    "",
    "1. Methods",
    "1.1 Data eligibility",
    "Images with visible motion blur were excluded from the study.",
    "Only photographs taken at 90 degrees to the occlusal plane were included.",
    "",
    "1.2 Preprocessing",
    "Images were converted to grayscale and normalized by percentile stretching between the 1st and 99th percentile.",
    "A Gaussian filter with sigma = 2 was applied to reduce noise.",
    "",
    "1.3 Segmentation",
    "A global threshold of 128 intensity units was applied to obtain the gingival mask.",
    "Morphological opening was then applied with a radius of 3 pixels.",
]
PAGE2 = [
    "1.4 Measurement",
    "The gingival area was computed as the pixel count multiplied by the squared pixel spacing of 0.05 mm.",
    "Formula: A = N x s^2 where N is the number of mask pixels.",
    "Supplement: The Gaussian blur (sigma = 2) was used to suppress sensor noise before thresholding.",
    "The margin ratio is calculated using the method of Smith et al.",
    "Pixels above 0.6 were retained.",
    "",
    "2. Validation",
    "Performance was evaluated on 40 images against manual annotation using the Dice coefficient.",
    "This method diagnoses periodontitis and should guide treatment decisions.",
]

if __name__ == "__main__":
    open("text-layer.pdf", "wb").write(text_pdf([PAGE1, PAGE2]))
    open("scanned.pdf", "wb").write(scanned_pdf(2))
