"""Planches de comparaison à partir des images du mode observation.

Usage : python3 -I outils/planches.py <dossier des PNG> <dossier de sortie>

Chaque image source couvre une tile de 1 km (1 pixel = 1 m, nord en haut) :
bâti en gris, barricades en rouge, densité de passage des zombies sur la
fenêtre (du jaune au violet), humains en vert, destination du repli en bleu.
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

src = Path(sys.argv[1])
dst = Path(sys.argv[2])
dst.mkdir(parents=True, exist_ok=True)
try:
    police = ImageFont.truetype("DejaVuSans.ttf", 12)
except OSError:
    police = ImageFont.load_default()


def ouvrir(nom):
    return Image.open(src / nom).convert("RGB")


def agrandir(im, k):
    # Agrandissement par pixels entiers (sans lissage).
    w, h = im.size
    out = Image.new("RGB", (w * k, h * k))
    px, po = im.load(), out.load()
    for y in range(h * k):
        for x in range(w * k):
            po[x, y] = px[x // k, y // k]
    return out


def planche(vignettes, colonnes, nom):
    w, h = vignettes[0][1].size
    marge, titre = 8, 18
    lignes = (len(vignettes) + colonnes - 1) // colonnes
    out = Image.new("RGB", (colonnes * (w + marge) - marge, lignes * (h + titre + marge) - marge), "white")
    d = ImageDraw.Draw(out)
    for k, (texte, im) in enumerate(vignettes):
        x = (k % colonnes) * (w + marge)
        y = (k // colonnes) * (h + titre + marge)
        d.text((x + 2, y + 2), texte, fill=(0, 0, 0), font=police)
        out.paste(im, (x, y + titre))
    out.save(dst / nom, optimize=True)
    print(dst / nom)


centre = (300, 250, 700, 650)
planche(
    [
        ("Ligne droite (P-01, test 1)", ouvrir("assaut-ligne-045s.png").crop(centre)),
        ("Champ 1 m", ouvrir("assaut-1m-045s.png").crop(centre)),
        ("Champ 2 m (retenu)", ouvrir("assaut-2m-045s.png").crop(centre)),
        ("Champ 5 m", ouvrir("assaut-5m-045s.png").crop(centre)),
    ],
    2,
    "assaut-resolutions.png",
)
planche([("Assaut, champ 2 m, 0 à 45 s", ouvrir("assaut-2m-045s.png").reduce(2))], 1, "assaut-tile.png")
zoom = (560, 460, 760, 660)
planche(
    [
        ("2 m sans marge", agrandir(ouvrir("assaut-2m-sans-marge-045s.png").crop(zoom), 2)),
        ("2 m avec marge (retenu)", agrandir(ouvrir("assaut-2m-045s.png").crop(zoom), 2)),
        ("5 m", agrandir(ouvrir("assaut-5m-045s.png").crop(zoom), 2)),
    ],
    3,
    "zoom-marge.png",
)
planche(
    [
        (f"Repli, {a} à {b} s", ouvrir(f"repli-p20-{b:03d}s.png").reduce(2))
        for a, b in [(0, 80), (80, 160), (160, 240)]
    ],
    3,
    "repli.png",
)
zb = (380, 380, 580, 580)
planche(
    [
        ("Assaut, 0 à 120 s", agrandir(ouvrir("barricade-120s.png").crop(zb), 2)),
        ("Barricades posées à 120 s : 120 à 240 s", agrandir(ouvrir("barricade-240s.png").crop(zb), 2)),
    ],
    2,
    "barricade.png",
)
