#!/usr/bin/env python3
"""P-08 — Générer ou stocker : extrapolation à la France entière.

Entrées :
  --manche, --paris : tiles-*.csv de `--mode stats` (une ligne par tile) ;
  --p07 : resultats-extrapolation.csv de P-07 (nombre de tiles par type).

Méthode : moyenne par type de tile (Manche ; « dense » : Manche et Paris
ensemble) de la taille du maillage stocké (encodage compact + zstd 9, ramené
au niveau 19 par le rapport mesuré sur une tile sur dix), de la section `bati`
de P-07 et du temps de génération sur un thread ; multipliée par le nombre de
tiles du type (P-07). Les tiles de mer et de côte n'ont pas de bâtiments.

Sortie : un tableau CSV sur la sortie standard, un résumé sur stderr.
"""

import argparse
import csv
import sys

TYPES = ["vide", "rural", "bourg", "urbain", "dense"]


def lire(f):
    return list(csv.DictReader(open(f)))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--manche", required=True)
    ap.add_argument("--paris", required=True)
    ap.add_argument("--p07", required=True)
    a = ap.parse_args()
    m = lire(a.manche)
    p = lire(a.paris)
    # Rapport zstd 19 / zstd 9 (tiles où les deux sont mesurés).
    z = [x for x in m + p if int(x["z19_iso"]) > 0]
    r19 = sum(int(x["z19_iso"]) for x in z) / sum(int(x["z9_iso"]) for x in z)
    par = {}
    for t in TYPES:
        v = [x for x in m if x["type"] == t]
        if t == "dense":
            v = v + [x for x in p if x["type"] == "dense"]
        n = len(v)
        moy = lambda c: sum(float(x[c]) for x in v) / n
        par[t] = {
            "n": n,
            "iso": moy("z9_iso") * r19,
            "loin": moy("z9_loin") * r19,
            "bati": moy("bati_p07"),
            "ms": moy("ms_gen"),
            "bat": moy("nb_bat"),
            "gpu": moy("gpu_iso"),
        }
    nb = {}
    for x in lire(a.p07):
        if x["type"] in TYPES:
            nb[x["type"]] = nb.get(x["type"], 0) + int(x["tiles"])
    w = csv.writer(sys.stdout)
    w.writerow(["type", "tiles_france", "tiles_mesurees", "batiments_moyen", "bati_p07_octets", "maillage_iso_octets", "maillage_loin_octets",
                "go_bati_p07", "go_maillages_iso", "go_maillages_iso_et_loin", "ms_generation_moyen", "heures_generation_1_thread"])
    tot = [0.0] * 5
    for t in TYPES:
        q = par[t]
        n = nb.get(t, 0)
        g = [n * q["bati"] / 1e9, n * q["iso"] / 1e9, n * (q["iso"] + q["loin"]) / 1e9, n * q["ms"] / 3.6e6]
        tot = [tot[0] + n] + [tot[k + 1] + g[k] for k in range(4)]
        w.writerow([t, n, q["n"], round(q["bat"]), round(q["bati"]), round(q["iso"]), round(q["loin"]), round(g[0], 2), round(g[1], 1), round(g[2], 1), round(q["ms"], 2), round(g[3], 2)])
    print(f"rapport zstd 19 / 9 : {r19:.3f} ({len(z)} tiles)", file=sys.stderr)
    print(f"tiles bâties (France, P-07) : {tot[0]:.0f}", file=sys.stderr)
    print(f"section bati de P-07 : {tot[1]:.1f} Go ; maillages stockés (vue isométrique) : {tot[2]:.0f} Go ; avec la vue lointaine : {tot[3]:.0f} Go", file=sys.stderr)
    print(f"génération de toute la France : {tot[4]:.1f} h sur un thread", file=sys.stderr)


if __name__ == "__main__":
    main()
