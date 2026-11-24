#!/usr/bin/env python3
"""P-05 — Extraction des bâtiments de la BD TOPO pour les colonies.

Python pur (sqlite3 de la bibliothèque standard) : un GeoPackage est une base
SQLite dont les géométries sont du WKB précédé d'un en-tête « GP ».

Usage : python3 -I extraire_batiments.py <fichier.gpkg> <dossier de sortie>

Sortie : batiments.csv, une ligne par bâtiment « En service », triée par
identifiant (fid) :
  fid, x, y            centroïde de l'emprise (m, Lambert-93)
  emprise              surface de l'emprise (m², trous déduits)
  hauteur              m (vide si absente)
  etages               nombre_d_etages (vide si absent)
  logements            nombre_de_logements (vide si absent)
  usage_1, usage_2     tels quels
  leger                construction_legere (0 / 1)

Attributs vérifiés dans le fichier (édition 2026-09-15) : hauteur (REAL),
nombre_d_etages et nombre_de_logements (MEDIUMINT, absents pour environ
30 % des bâtiments), usage_1 et usage_2 (Résidentiel, Indifférencié,
Annexe, Commercial et services, Industriel, Sportif, Religieux, Agricole).
"""

import csv
import os
import struct
import sqlite3
import sys


def anneaux(blob):
    """Anneaux (listes de (x, y)) de chaque polygone d'une géométrie GPKG."""
    assert blob[:2] == b"GP"
    flags = blob[3]
    off = 8 + {0: 0, 1: 32, 2: 48, 3: 48, 4: 64}[(flags >> 1) & 7]
    polys = []

    def lire(off):
        bo = "<" if blob[off] == 1 else ">"
        (t,) = struct.unpack_from(bo + "I", blob, off + 1)
        off += 5
        dims = 2
        if t & 0x80000000 or (t % 10000) // 1000 in (1, 2):
            dims = 3
        if (t & 0xC0000000) == 0xC0000000 or (t % 10000) // 1000 == 3:
            dims = 4
        base = (t & 0xFFFF) % 1000
        if base == 3:
            (nr,) = struct.unpack_from(bo + "I", blob, off)
            off += 4
            poly = []
            for _ in range(nr):
                (n,) = struct.unpack_from(bo + "I", blob, off)
                off += 4
                pts = [struct.unpack_from(bo + "dd", blob, off + k * 8 * dims) for k in range(n)]
                off += n * 8 * dims
                poly.append(pts)
            polys.append(poly)
        elif base == 6:
            (n,) = struct.unpack_from(bo + "I", blob, off)
            off += 4
            for _ in range(n):
                off = lire(off)
        else:
            raise ValueError(f"type WKB inattendu : {t}")
        return off

    lire(off)
    return polys


def surface_centroide(polys):
    """Surface (trous déduits) et centroïde, par la formule du lacet."""
    a_tot, cx, cy = 0.0, 0.0, 0.0
    for poly in polys:
        for k, pts in enumerate(poly):
            x0, y0 = pts[0]
            a = sx = sy = 0.0
            for (ax, ay), (bx, by) in zip(pts, pts[1:] + pts[:1]):
                ax, ay, bx, by = ax - x0, ay - y0, bx - x0, by - y0
                c = ax * by - bx * ay
                a += c
                sx += (ax + bx) * c
                sy += (ay + by) * c
            a /= 2.0
            if a == 0.0:
                continue
            gx, gy = sx / (6.0 * a) + x0, sy / (6.0 * a) + y0
            # Anneau extérieur compté positif, trous négatifs, quel que soit
            # le sens de parcours.
            s = abs(a) if k == 0 else -abs(a)
            a_tot += s
            cx += gx * s
            cy += gy * s
    if a_tot <= 0.0:
        return 0.0, None
    return a_tot, (cx / a_tot, cy / a_tot)


def main():
    gpkg, sortie = sys.argv[1], sys.argv[2]
    os.makedirs(sortie, exist_ok=True)
    c = sqlite3.connect(f"file:{gpkg}?mode=ro", uri=True)
    n = 0
    with open(os.path.join(sortie, "batiments.csv"), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["fid", "x", "y", "emprise", "hauteur", "etages", "logements", "usage_1", "usage_2", "leger"])
        for fid, blob, h, et, lg, u1, u2, leger in c.execute(
            "select fid, geometrie, hauteur, nombre_d_etages, nombre_de_logements, usage_1, usage_2, "
            "construction_legere from batiment where etat_de_l_objet = 'En service' order by fid"
        ):
            s, g = surface_centroide(anneaux(blob))
            if g is None:
                continue
            w.writerow([
                fid, f"{g[0]:.2f}", f"{g[1]:.2f}", f"{s:.1f}",
                "" if h is None else f"{h:.1f}",
                "" if et is None else et,
                "" if lg is None else lg,
                u1 or "", u2 or "", int(bool(leger)),
            ])
            n += 1
            if n % 100000 == 0:
                print(n, file=sys.stderr)
    print(f"{n} bâtiments", file=sys.stderr)


if __name__ == "__main__":
    main()
