#!/usr/bin/env python3
"""P-07 — Extrapolation à la France entière.

Entrées :
  --manche  tiles.csv et etapes.csv de la campagne sur la Manche ;
  --paris   tiles.csv de la même chaîne sur Paris (tiles denses) ;
  --carreaux carreaux_1km_met.csv (INSEE, Filosofi 2021, national) ;
  --sources tailles-sources.csv (tailles_sources.py).

Méthode :
  1. Taille et temps de calcul moyens par type de tile, mesurés sur la Manche
     (et Paris pour « dense », plus nombreux à Paris).
  2. Nombre de tiles de chaque type en métropole : les carreaux de 1 km de
     l'INSEE donnent les tiles habitées par classe d'habitants (grille LAEA,
     même surface) ; les tiles non habitées = surface terrestre − habitées.
  3. Outre-mer : surface terrestre, réparties comme la Manche hors « dense »
     (hypothèse grossière, faute de carreaux pour la plupart des territoires).
  4. Mer à 30 km : tiles de mer de la Manche par km de trait de côte,
     rapportées au trait de côte national (valeurs à vérifier).
  5. Temps : la lecture et le relief sont proportionnels au volume des
     sources (mesuré sur la Manche, rapporté aux tailles de toutes les
     archives) ; les passes par tile au nombre de tiles de chaque type.

Sortie : un tableau CSV sur la sortie standard, et un résumé sur stderr.
"""

import argparse
import csv
import statistics as st
import sys

CLASSES = ["vide", "rural", "bourg", "urbain", "dense"]
# Hypothèses (à vérifier) : surfaces terrestres en km².
SURFACE_METRO = 543_940  # métropole, Corse comprise (IGN)
SURFACE_OUTREMER = {
    "Guadeloupe": 1_628, "Martinique": 1_128, "Guyane": 83_534, "Réunion": 2_512, "Mayotte": 374,
    "Saint-Pierre-et-Miquelon": 242, "Saint-Martin": 53, "Saint-Barthélemy": 25,
}  # le Pacifique n'est pas dans le jeu (décision prise pendant P-07)
# Traits de côte (km, à vérifier) : métropole ~ 5 500 (au 1:100 000, plus
# long au détail de la BD TOPO) ; outre-mer hors Pacifique, ordre de grandeur.
COTE_METRO_KM = 5_500
COTE_OUTREMER_KM = 2_000  # Antilles, Guyane, Réunion, Mayotte, Saint-Pierre-et-Miquelon
# Trait de côte de la Manche au même niveau de détail : environ 350 km
# (hors zone tampon ; la BD TOPO en donne 692 km zone tampon comprise).
COTE_MANCHE_KM = 350

ETAPES_TILE = ["ms_bati", "ms_franch", "ms_communes", "ms_occupation", "ms_composantes", "ms_encodage", "ms_zone", "ms_assemblage"]  # le relief est extrapolé au volume des sources


def classe(h):
    h = int(h)
    if h == 0:
        return "vide"
    if h < 100:
        return "rural"
    if h < 1000:
        return "bourg"
    if h < 5000:
        return "urbain"
    return "dense"


def lire_tiles(chemin):
    with open(chemin) as f:
        return list(csv.DictReader(f))


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--manche", required=True)
    a.add_argument("--etapes", required=True)
    a.add_argument("--paris", required=True)
    a.add_argument("--carreaux", required=True)
    a.add_argument("--sources", required=True)
    a.add_argument("--threads", type=int, default=16)
    o = a.parse_args()

    manche = lire_tiles(o.manche)
    paris = lire_tiles(o.paris)
    par_type = {}
    for t in manche + [t for t in paris if t["type"] == "dense"]:
        ty = t["type"]
        if ty == "cote":
            ty = "vide"
        par_type.setdefault(ty, []).append(t)

    # Paris n'a pas de relief (pas de RGE ALTI dans les sources) : ses tiles
    # reçoivent le relief moyen des tiles urbaines de la Manche.
    rel_urbain = st.mean(int(t["relief_z"]) for t in manche if t["type"] == "urbain")

    def taille(ts):
        return st.mean(int(t["octets"]) + (rel_urbain if int(t.get("relief_z", "0")) == 0 and t["type"] == "dense" else 0) for t in ts)

    def calcul(ts):
        return st.mean(sum(float(t[e]) for e in ETAPES_TILE) for t in ts)

    # Comptes nationaux de tiles habitées.
    hab = {c: 0 for c in CLASSES}
    pop = 0.0
    with open(o.carreaux) as f:
        for r in csv.DictReader(f):
            v = float(r["ind"])
            pop += v
            hab[classe(round(v))] += 1
    habitees = sum(hab[c] for c in CLASSES if c != "vide")
    comptes = dict(hab)
    comptes["vide"] = max(0, SURFACE_METRO - habitees)

    # Outre-mer : répartition de la Manche hors dense.
    terre_manche = [t for t in manche if t["terre"] == "1"]
    rep = {c: sum(1 for t in terre_manche if (t["type"] if t["type"] != "cote" else "vide") == c) / len(terre_manche) for c in CLASSES}
    om = sum(SURFACE_OUTREMER.values())
    comptes_om = {c: round(om * rep[c]) for c in CLASSES}

    # Mer.
    mer_manche = sum(1 for t in manche if t["type"] == "mer")
    mer_par_km = mer_manche / COTE_MANCHE_KM
    mer = round(mer_par_km * (COTE_METRO_KM + COTE_OUTREMER_KM))

    w = csv.writer(sys.stdout)
    w.writerow(["perimetre", "type", "tiles", "octets_moyen", "go_total", "ms_calcul_moyen", "heures_calcul_1_thread"])
    tot_o, tot_ms, tot_n = 0.0, 0.0, 0
    for nom, cs in [("metropole", comptes), ("outre-mer", comptes_om)]:
        for c in CLASSES:
            if c not in par_type:
                continue
            n = cs[c]
            so, sm = taille(par_type[c]), calcul(par_type[c])
            tot_o += n * so
            tot_ms += n * sm
            tot_n += n
            w.writerow([nom, c, n, round(so), round(n * so / 1e9, 2), round(sm, 2), round(n * sm / 3.6e6, 2)])
    if "mer" not in par_type:  # mode essai
        par_type["mer"] = [{"octets": "0", **{e: "0" for e in ETAPES_TILE}}]
    so, sm = taille(par_type["mer"]), calcul(par_type["mer"])
    w.writerow(["mer-30km", "mer", mer, round(so), round(mer * so / 1e9, 2), round(sm, 2), round(mer * sm / 3.6e6, 2)])
    tot_o += mer * so
    tot_ms += mer * sm
    tot_n += mer

    # Sources.
    src = {"BDTOPO": 0, "1M": 0, "5M": 0}
    ed_manche = {}
    with open(o.sources) as f:
        for r in csv.DictReader(f):
            n = int(r["octets"] or 0)
            k = "BDTOPO" if r["ressource"] == "BDTOPO" else ("1M" if "_1M_" in r["edition"] else "5M")
            src[k] += n
            if r["zone"] == "D050":
                ed_manche[k] = n
    # Temps de lecture et de relief, proportionnels au volume des sources.
    etapes = {}
    with open(o.etapes) as f:
        for r in csv.DictReader(f):
            etapes[r["etape"]] = float(r["s"])
    k_bd = src["BDTOPO"] / ed_manche.get("BDTOPO", 1)
    k_rel = src["1M"] / ed_manche.get("1M", 1)
    t_lecture = etapes["lecture"] * k_bd
    t_relief = etapes["relief"] * k_rel
    t_tiles = tot_ms / 1e3 / o.threads
    t_autres = sum(etapes.get(e, 0) for e in ["inventaire", "population", "repartition", "fusion", "ecriture"]) * k_bd
    print(f"tiles : {tot_n} (dont mer {mer}) ; sortie {tot_o / 1e9:.1f} Go", file=sys.stderr)
    print(f"habitants (carreaux 1 km, métropole) : {pop:,.0f} ; tiles habitées : {habitees}", file=sys.stderr)
    print(f"sources : BD TOPO {src['BDTOPO'] / 1e9:.1f} Go, RGE ALTI 1 m {src['1M'] / 1e9:.1f} Go, 5 m {src['5M'] / 1e9:.1f} Go (facteurs {k_bd:.0f} et {k_rel:.0f} sur la Manche)", file=sys.stderr)
    print(
        f"durée sur {o.threads} threads : lecture {t_lecture / 3600:.2f} h, relief {t_relief / 3600:.2f} h, "
        f"passes par tile {t_tiles / 3600:.2f} h, autres {t_autres / 3600:.2f} h ; total {(t_lecture + t_relief + t_tiles + t_autres) / 3600:.1f} h",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
