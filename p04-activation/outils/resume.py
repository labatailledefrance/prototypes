"""Résumé des résultats de P-04 (tables du README).

Usage : python3 -I outils/resume.py [resultats.csv]
"""
import csv
import statistics as S
import sys
from pathlib import Path

ici = Path(__file__).resolve().parent.parent
fichier = Path(sys.argv[1]) if len(sys.argv) > 1 else ici / "resultats.csv"

COLS = [
    ("scenario", "sc"), ("bord", "bord"), ("densite", "d"), ("vitesse", "v"), ("delai_s", "délai"),
    ("etalement", "étal"), ("zone", "zone"), ("lot_naissance", "lotN"), ("threads", "th"),
    ("lot_min", "lm"), ("duree_s", "s"), ("mediane_ms", "méd"), ("p99_ms", "p99"),
    ("p999_ms", "p999"), ("max_ms", "max"), ("max_chantier_tick_ms", "chMax"),
    ("max_entites_tick_ms", "entMax"), ("ticks_sur_budget", ">bud"), ("ticks_sur_2_5ms", ">2.5"),
    ("p99_avec_chantier_ms", "p99ch"), ("max_avec_chantier_ms", "maxch"), ("p99_sans_chantier_ms", "p99sans"),
    ("actifs_moy", "act"), ("actifs_max", "actM"), ("activations", "nA"), ("restaurations", "nR"),
    ("desactivations", "nD"), ("activations_min", "A/min"), ("desactivations_min", "D/min"),
    ("chantier_ticks_moy", "chT"), ("chantier_ticks_max", "chTM"), ("chantier_ms_moy", "chMs"),
    ("chantier_ms_max", "chMsM"), ("naissance_ms_moy", "naiss"), ("naissance_ms_max", "naissM"),
    ("instantane_ms_moy", "inst"), ("destruction_ms_moy", "destr"), ("destruction_ms_max", "destrM"),
    ("encode_ms_moy", "enc"), ("ecriture_ms_moy", "écr"), ("lecture_ms_moy", "lect"), ("decode_ms_moy", "déc"),
    ("octets_moy", "octets"), ("controles_ok", "ctrlOK"), ("controles_ko", "ctrlKO"),
    ("constructions_ok", "cstOK"), ("constructions_ko", "cstKO"), ("passages", "pass"),
    ("passages_max_tick", "passM"), ("vers_chantier", "→ch"), ("vers_inerte", "→in"), ("migres", "migr"),
    ("abattus", "abat"), ("conservation_ko", "consKO"), ("vivants_fin", "viv"), ("rss_mo", "rss"), ("somme", "somme"),
]

lignes = list(csv.DictReader(open(fichier)))
print("\t".join(c[1] for c in COLS))
for r in lignes:
    print("\t".join(r.get(c[0], "") for c in COLS))
