"""Download the official SAM ViT-B checkpoint into ``<Rosaray>/model/sam/``.

    python -m crr.download_model [--output PATH]

An existing file is reused only if its SHA-256 matches the official digest.
"""

from __future__ import annotations

import argparse
import hashlib
import os
import urllib.request
from pathlib import Path

from .sam import default_checkpoint

URL = "https://dl.fbaipublicfiles.com/segment_anything/sam_vit_b_01ec64.pth"
SHA256 = "ec2df62732614e57411cdcf32a23ffdf28910380d03139ee0f4fcbe91eb8c912"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download(dest: Path) -> bool:
    """Returns True if downloaded, False if an existing valid file was reused."""
    if dest.is_file() and sha256_file(dest) == SHA256:
        return False
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    try:
        with urllib.request.urlopen(URL) as resp, tmp.open("wb") as out:
            while True:
                chunk = resp.read(1 << 20)
                if not chunk:
                    break
                out.write(chunk)
        actual = sha256_file(tmp)
        if actual != SHA256:
            raise ValueError(f"SHA-256 mismatch: expected {SHA256}, got {actual}")
        os.replace(tmp, dest)
        return True
    finally:
        if tmp.exists():
            tmp.unlink()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=default_checkpoint())
    args = parser.parse_args()
    print(("Downloaded: " if download(args.output) else "Already verified: ") + str(args.output))


if __name__ == "__main__":
    main()
