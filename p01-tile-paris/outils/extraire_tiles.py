#!/usr/bin/env python3
"""P-01 — Extraction des emprises réelles de la BD TOPO (bâtiments) en
bitmaps de 1 m par tile de 1 km alignée sur le Lambert-93, et décompte des
tiles le long du boulevard périphérique.

Python pur (sqlite3 de la bibliothèque standard) : un GeoPackage est une base
SQLite dont les géométries sont du WKB précédé d'un en-tête « GP ».

Usage : python3 -I extraire_tiles.py <fichier.gpkg> <dossier de sortie>

Sortie :
  x<X>_y<Y>.bin   bitmap de la tile (X, Y en km Lambert-93) : 1 000 × 1 000
                  bits, case (i, j) au bit j*1000+i, mots u64 petit-boutiens,
                  j croissant vers le nord ;
  tiles.csv       x_km, y_km, couverture, dans_peripherique ;
  peripherique.txt  décompte des tiles actives autour du périphérique.
"""

import math
import os
import sqlite3
import struct
import sys

COTE = 1000
CENTRE_PARIS = (652_000.0, 6_862_000.0)  # Île de la Cité, approximatif


def wkb_polygones(blob):
    """Décode une géométrie GeoPackage en liste de polygones (listes
    d'anneaux, chaque anneau une liste de (x, y))."""
    assert blob[:2] == b"GP", "en-tête GeoPackage attendu"
    flags = blob[3]
    env = (flags >> 1) & 7
    taille_env = {0: 0, 1: 32, 2: 48, 3: 48, 4: 64}[env]
    off = 8 + taille_env
    polys = []

    def lire(off):
        bo = "<" if blob[off] == 1 else ">"
        (t,) = struct.unpack_from(bo + "I", blob, off + 1)
        off += 5
        # Dimensions : ISO (1000, 2000, 3000) ou EWKB (bits de poids fort).
        dims = 2
        if t & 0x80000000 or (t % 10000) // 1000 in (1, 2):
            dims = 3
        if (t & 0xC0000000) == 0xC0000000 or (t % 10000) // 1000 == 3:
            dims = 4
        base = (t & 0xFFFF) % 1000
        if base == 3:  # Polygon
            (nr,) = struct.unpack_from(bo + "I", blob, off)
            off += 4
            anneaux = []
            for _ in range(nr):
                (npts,) = struct.unpack_from(bo + "I", blob, off)
                off += 4
                pts = []
                for _ in range(npts):
                    x, y = struct.unpack_from(bo + "dd", blob, off)
                    pts.append((x, y))
                    off += 8 * dims
                anneaux.append(pts)
            polys.append(anneaux)
        elif base == 6:  # MultiPolygon
            (n,) = struct.unpack_from(bo + "I", blob, off)
            off += 4
            for _ in range(n):
                off = lire(off)
        else:
            raise ValueError(f"type WKB inattendu : {t}")
        return off

    lire(off)
    return polys


def remplir(bits, poly, x0, y0):
    """Pose à 1 les cases de la tile (origine x0, y0) dont le centre est dans
    le polygone (règle pair-impair : les cours intérieures restent libres)."""
    aretes = []
    ymin, ymax = math.inf, -math.inf
    for anneau in poly:
        for (ax, ay), (bx, by) in zip(anneau, anneau[1:] + anneau[:1]):
            ax, ay, bx, by = ax - x0, ay - y0, bx - x0, by - y0
            if ay != by:
                aretes.append((ax, ay, bx, by))
            ymin, ymax = min(ymin, ay, by), max(ymax, ay, by)
    j0 = max(0, math.ceil(ymin - 0.5))
    j1 = min(COTE - 1, math.floor(ymax - 0.5))
    for j in range(j0, j1 + 1):
        yc = j + 0.5
        xs = []
        for ax, ay, bx, by in aretes:
            if (ay <= yc) != (by <= yc):
                xs.append(ax + (yc - ay) * (bx - ax) / (by - ay))
        xs.sort()
        for k in range(0, len(xs) - 1, 2):
            i0 = max(0, math.ceil(xs[k] - 0.5))
            i1 = min(COTE - 1, math.floor(xs[k + 1] - 0.5))
            base = j * COTE
            for i in range(i0, i1 + 1):
                bits[base + i] = 1


def tiles_peripherique(c):
    """Tiles traversées par le périphérique (échantillonnage tous les 5 m)."""
    points = []
    for (blob,) in c.execute(
        "select geometrie from troncon_de_route "
        "where cpx_toponyme_route_nommee like '%ériph%' "
        "and cpx_toponyme_route_nommee not like '%Bretelle%'"
    ):
        flags = blob[3]
        off = 8 + {0: 0, 1: 32, 2: 48, 3: 48, 4: 64}[(flags >> 1) & 7]
        bo = "<" if blob[off] == 1 else ">"
        (t,) = struct.unpack_from(bo + "I", blob, off + 1)
        dims = 3 if (t & 0x80000000 or (t % 10000) // 1000 in (1, 2)) else 2
        (n,) = struct.unpack_from(bo + "I", blob, off + 5)
        pts = [struct.unpack_from(bo + "dd", blob, off + 9 + k * 8 * dims) for k in range(n)]
        for (ax, ay), (bx, by) in zip(pts, pts[1:]):
            pas = max(1, int(math.hypot(bx - ax, by - ay) / 5))
            for s in range(pas + 1):
                points.append((ax + (bx - ax) * s / pas, ay + (by - ay) * s / pas))
    return points


def main():
    gpkg, sortie = sys.argv[1], sys.argv[2]
    os.makedirs(sortie, exist_ok=True)
    c = sqlite3.connect(f"file:{gpkg}?mode=ro", uri=True)

    # 1. Périphérique : tiles traversées, voisines intérieures et extérieures.
    pts = tiles_peripherique(c)
    traversees = {(int(x // 1000), int(y // 1000)) for x, y in pts}

    def rayon_peri(cx, cy):
        # Distance au centre du point du périphérique le plus proche en angle.
        a = math.atan2(cy - CENTRE_PARIS[1], cx - CENTRE_PARIS[0])
        best = min(pts, key=lambda p: abs(math.remainder(
            math.atan2(p[1] - CENTRE_PARIS[1], p[0] - CENTRE_PARIS[0]) - a, math.tau)))
        return math.hypot(best[0] - CENTRE_PARIS[0], best[1] - CENTRE_PARIS[1])

    def interieure(tx, ty):
        cx, cy = tx * 1000 + 500, ty * 1000 + 500
        return math.hypot(cx - CENTRE_PARIS[0], cy - CENTRE_PARIS[1]) < rayon_peri(cx, cy)

    voisines = {(x + dx, y + dy) for x, y in traversees
                for dx in (-1, 0, 1) for dy in (-1, 0, 1)} - traversees
    v_int = {t for t in voisines if interieure(*t)}
    v_ext = voisines - v_int
    longueur = sum(math.hypot(b[0] - a[0], b[1] - a[1]) for a, b in zip(pts, pts[1:])
                   if math.hypot(b[0] - a[0], b[1] - a[1]) < 50) / 2  # deux sens
    with open(os.path.join(sortie, "peripherique.txt"), "w") as f:
        f.write(f"longueur_km_approx {longueur / 1000:.1f}\n")
        f.write(f"tiles_traversees {len(traversees)}\n")
        f.write(f"voisines_interieures {len(v_int)}\n")
        f.write(f"voisines_exterieures {len(v_ext)}\n")
        f.write(f"total_actives {len(traversees) + len(voisines)}\n")
    print(open(os.path.join(sortie, "peripherique.txt")).read(), end="")

    # 2. Bitmaps des tiles de Paris intra-muros (centre à l'intérieur du
    #    périphérique, ou traversées par lui).
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    candidates = [(tx, ty)
                  for tx in range(int(min(xs) // 1000), int(max(xs) // 1000) + 1)
                  for ty in range(int(min(ys) // 1000), int(max(ys) // 1000) + 1)
                  if (tx, ty) in traversees or interieure(tx, ty)]
    lignes = []
    for tx, ty in candidates:
        x0, y0 = tx * 1000, ty * 1000
        bits = bytearray(COTE * COTE)
        for (blob,) in c.execute(
            "select b.geometrie from batiment b join rtree_batiment_geometrie r on b.fid = r.id "
            "where r.maxx >= ? and r.minx <= ? and r.maxy >= ? and r.miny <= ? "
            "and b.etat_de_l_objet = 'En service'",
            (x0, x0 + COTE, y0, y0 + COTE),
        ):
            for poly in wkb_polygones(blob):
                remplir(bits, poly, x0, y0)
        couverture = sum(bits) / (COTE * COTE)
        # Empaquetage : bit k du mot k // 64.
        mots = bytearray(8 * (COTE * COTE // 64 + 1))
        for k in range(0, COTE * COTE, 64):
            v = 0
            for b, val in enumerate(bits[k:k + 64]):
                v |= val << b
            struct.pack_into("<Q", mots, 8 * (k // 64), v)
        with open(os.path.join(sortie, f"x{tx}_y{ty}.bin"), "wb") as f:
            f.write(mots)
        dans = (tx, ty) not in traversees
        lignes.append((tx, ty, couverture, dans))
        print(f"tile {tx} {ty} : {couverture:.3f}", file=sys.stderr)
    lignes.sort(key=lambda l: -l[2])
    with open(os.path.join(sortie, "tiles.csv"), "w") as f:
        f.write("x_km,y_km,couverture,dans_peripherique\n")
        for tx, ty, cov, dans in lignes:
            f.write(f"{tx},{ty},{cov:.4f},{int(dans)}\n")


if __name__ == "__main__":
    main()
