"""Convertit les images PPM du mode observation en PNG (et supprime les PPM).

Usage : python3 -I outils/ppm_vers_png.py <dossier> [côté en pixels]
"""
import sys
from pathlib import Path

from PIL import Image

dossier = Path(sys.argv[1])
cote = int(sys.argv[2]) if len(sys.argv) > 2 else 1000
for ppm in sorted(dossier.glob("*.ppm")):
    img = Image.open(ppm)
    if cote != img.width:
        img = img.resize((cote, cote), Image.LANCZOS)
    img.save(ppm.with_suffix(".png"), optimize=True)
    ppm.unlink()
    print(ppm.with_suffix(".png"))
