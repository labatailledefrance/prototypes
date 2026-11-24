#!/usr/bin/env python3
"""P-07 — Tailles des sources IGN pour toute la France (métadonnées seulement,
aucun téléchargement de données).

Interroge le service de téléchargement de la Géoplateforme : pour chaque
département (métropole et outre-mer), la dernière édition de la BD TOPO
(GeoPackage, tous thèmes) et du RGE ALTI (1 m et 5 m, ASC), et la taille de
leurs archives.

Usage : python3 -I tailles_sources.py > tailles-sources.csv
"""

import re
import sys
import urllib.request

API = "https://data.geopf.fr/telechargement/resource"


def lire(url):
    req = urllib.request.Request(url, headers={"User-Agent": "prototype-p07"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read().decode()


def entrees(url):
    t = lire(url)
    return [
        (re.findall(r"<title>(.*?)</title>", e), re.findall(r'href="([^"]+)"', e), re.findall(r'length="(\d+)"', e))
        for e in re.findall(r"<entry>(.*?)</entry>", t, re.S)
    ]


def zones():
    t = lire(f"{API}/BDTOPO?format=GPKG&limit=1")
    return sorted(set(re.findall(r'gpf_dl:zone term="(D\d\w\w?)"', t)))


def derniere(ressource, zone, motif):
    """Dernière édition dont le titre correspond au motif, et sa taille totale."""
    meilleur = None
    page = 1
    while True:
        es = entrees(f"{API}/{ressource}?zone={zone}&limit=50&page={page}")
        for titres, _, _ in es:
            for ti in titres:
                if re.search(motif, ti):
                    if meilleur is None or ti[-10:] > meilleur[-10:]:
                        meilleur = ti
        if len(es) < 50:
            break
        page += 1
    if meilleur is None:
        return None, 0
    total = 0
    for _, liens, tailles in entrees(f"{API}/{ressource}/{meilleur}"):
        for l, s in zip(liens, tailles):
            if not l.endswith(".md5"):
                total += int(s)
    return meilleur, total


def main():
    print("zone,ressource,edition,octets")
    for z in zones():
        for ressource, motif in [
            ("BDTOPO", r"BDTOPO_3-\d_TOUSTHEMES_GPKG_"),
            ("RGEALTI", r"RGEALTI_2-0_1M_ASC_"),
            ("RGEALTI", r"RGEALTI_2-0_5M_ASC_"),
        ]:
            try:
                ed, n = derniere(ressource, z, motif)
            except Exception as e:  # noqa: BLE001
                print(f"{z} {ressource} : {e}", file=sys.stderr)
                ed, n = None, 0
            print(f"{z},{ressource},{ed or ''},{n}", flush=True)


if __name__ == "__main__":
    main()
