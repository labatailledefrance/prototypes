#!/usr/bin/env bash
# P-04 — campagne de mesure. Écrit :
#   resultats-precalcul.csv : chaîne de données (zones accessibles du bloc) ;
#   resultats-banc.csv      : banc tile par tile (activation, sauvegarde) ;
#   resultats.csv           : simulations (sections C à H) ;
#   chrono-*.csv            : chronologies par seconde de jeu.
# Usage : ./run.sh [sections]   (par défaut : A B C D E F G H I)
#         ESSAI=1 ./run.sh ...  : chaque commande en version minuscule, dans
#                                 des fichiers « essai-* », pour valider les
#                                 arguments en quelques secondes.
# Durée : environ 100 min sur la machine de référence. Ne rien lancer de
# lourd en parallèle.
# Prérequis : tiles réelles dans ../../data/generated/p01-tiles (P-01).
set -euo pipefail
cd "$(dirname "$0")"
# `cargo test` ne reconstruit pas le binaire release : on le fait ici.
cargo build --release --quiet
bin=target/release/p04-activation
sections=("$@")
[[ ${#sections[@]} -eq 0 ]] && sections=(A B C D E F G H I)
essai=${ESSAI:-}
log=run.log
[[ -n $essai ]] && log=run-essai.log
: >> "$log"

horodater() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$log" >&2; }
voulu() { [[ " ${sections[*]} " == *" $1 "* ]]; }
f() { if [[ -n $essai ]]; then echo "essai-$1"; else echo "$1"; fi; }
out=$(f resultats.csv)
if [[ ! -s $out ]]; then "$bin" --entete > "$out"; fi

# mesurer <options...> : une simulation, une ligne de resultats.csv. En
# essai : 2 s de jeu.
mesurer() {
  local args=("$@")
  if [[ -n $essai ]]; then
    local filtre=() saute=0
    for x in "${args[@]}"; do
      if [[ $saute == 1 ]]; then saute=0; continue; fi
      if [[ $x == --duree ]]; then saute=1; continue; fi
      if [[ $x == --chrono ]]; then saute=1; continue; fi
      filtre+=("$x")
    done
    args=("${filtre[@]}" --duree 2)
  fi
  "$bin" "${args[@]}" >> "$out" 2>> "$log"
}

# ------------------------------------------------- A. Chaîne de données
if voulu A; then
  horodater "A. précalcul des zones accessibles"
  "$bin" --precalculer --entete > "$(f resultats-precalcul.csv)"
  "$bin" --precalculer >> "$(f resultats-precalcul.csv)" 2>> "$log"
fi

# ------------------------------------------------------------ B. Banc
if voulu B; then
  horodater "B. banc tile par tile"
  t=55; r=3; [[ -n $essai ]] && t=1 && r=1
  "$bin" --banc --entete > "$(f resultats-banc.csv)"
  "$bin" --banc --banc-tiles "$t" --repetitions "$r" >> "$(f resultats-banc.csv)" 2>> "$log"
fi

# ----------------------------------------- C. Colonne à ×1, Paris dense
if voulu C; then
  # D'un seul tenant : chantier en un tick, entités d'un coup, toutes les
  # désactivations du tick à la fois.
  TOUT=(--sans-etalement-activation --lot-naissance 0 --lot-destruction 0 --desactivations-par-tick 0)
  ENTITES=(--lot-naissance 0 --lot-destruction 0 --desactivations-par-tick 0)
  horodater "C. colonne, traversée complète (configuration retenue)"
  mesurer --scenario colonne --chrono "$(f chrono-colonne.csv)"
  horodater "C. colonne, traversée complète, d'un seul tenant"
  mesurer --scenario colonne "${TOUT[@]}"
  horodater "C. colonne, traversée complète, entités d'un coup"
  mesurer --scenario colonne "${ENTITES[@]}"
  horodater "C. colonne, traversée complète, sans préchauffage des tables"
  mesurer --scenario colonne --prechauffe-entites 0
  horodater "C. colonne, variantes (20 min de jeu)"
  mesurer --scenario colonne --duree 1200 --zone-locale
  mesurer --scenario colonne --duree 1200 --zone-locale --sans-etalement-activation
  mesurer --scenario colonne --duree 1200 --bord-traversable
  mesurer --scenario colonne --vitesse 10
fi

# ------------------------------- D. Colonne au budget de ×10, banlieue
if voulu D; then
  TOUT=(--sans-etalement-activation --lot-naissance 0 --lot-destruction 0 --desactivations-par-tick 0)
  horodater "D. colonne, densité 0,1 et 0,25, budget 2,5 ms"
  for d in 0.1 0.25; do
    mesurer --scenario colonne --densite "$d" --budget 2.5 --chrono "$(f chrono-colonne-$d.csv)"
    mesurer --scenario colonne --densite "$d" --budget 2.5 "${TOUT[@]}"
  done
fi

# ------------------------------------- E. Allers-retours en limite de tile
if voulu E; then
  for dl in 0 10 30 60 120 300; do
    horodater "E. base et patrouille, délai $dl s"
    mesurer --scenario base --humains 200 --delai "$dl" --duree 1800
  done
  horodater "E. base, délai 0, fsync"
  mesurer --scenario base --humains 200 --delai 0 --duree 1800 --fsync
  for pz in 20 180; do
    mesurer --scenario base --humains 200 --delai 60 --pause "$pz" --duree 1800
  done
fi

# -------------------------------------- F. Base avancée abandonnée, reprise
if voulu F; then
  horodater "F. reprise"
  mesurer --scenario reprise --humains 300 --chrono "$(f chrono-reprise.csv)"
fi

# --------------------------------------------- G. Migration entre inertes
if voulu G; then
  horodater "G. migration"
  mesurer --scenario migration --chrono "$(f chrono-migration.csv)"
fi

# ----------------------------------------------------- H. Déterminisme
if voulu H; then
  for th in 16 8 4 1; do
    horodater "H. déterminisme, $th threads"
    mesurer --scenario reprise --humains 300 --densite 0.25 --threads "$th"
  done
  mesurer --scenario reprise --humains 300 --densite 0.25 --lot-min 64
  mesurer --scenario reprise --humains 300 --densite 0.25 --lot-min 100000
fi
# ---------------------------------- I. Budgets du chantier pour ×10
# Budgets divisés par 4 : le chantier dure 4 fois plus de ticks, donc le même
# temps réel qu'à ×1 avec les budgets par défaut.
if voulu I; then
  X10=(--budget-terrain 25000 --budget-placement 1000 --lot-naissance 500 --lot-destruction 500)
  for d in 0.1 0.25; do
    horodater "I. colonne, densité $d, budget 2,5 ms, budgets du chantier ÷ 4"
    mesurer --scenario colonne --densite "$d" --budget 2.5 "${X10[@]}" --chrono "$(f chrono-colonne-$d-x10.csv)"
  done
fi
horodater "fin"
