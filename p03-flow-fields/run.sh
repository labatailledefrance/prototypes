#!/usr/bin/env bash
# P-03 — campagne de mesure. Écrit :
#   resultats-champs.csv      : banc des champs (calcul, recalcul, obstacles) ;
#   resultats-frontieres.csv  : banc des frontières (3 × 3 tiles) ;
#   resultats-voisinage.csv   : banc des index de zombies (un thread) ;
#   resultats.csv             : simulation complète (sections B à I) ;
#   resultats-p02.csv         : référence P-02 (son binaire, vue de 400 m).
# Usage : ./run.sh [sections]   (par défaut : A B C D E F G H I)
#         ESSAI=1 ./run.sh ...  : chaque commande en version minuscule, pour
#                                 valider les arguments avant la campagne.
# Durée : environ 55 min sur la machine de référence. Ne rien lancer de
# lourd en parallèle.
# Prérequis : tiles réelles dans ../../data/generated/p01-tiles (P-01, test 2)
# et le binaire de P-02 (../p02-perception/target/release/p02-perception).
set -euo pipefail
cd "$(dirname "$0")"
# `cargo test` ne reconstruit pas le binaire release : on le fait ici.
cargo build --release --quiet
bin=target/release/p03-flow-fields
p02=../p02-perception/target/release/p02-perception
log=run.log
sections=("$@")
[[ ${#sections[@]} -eq 0 ]] && sections=(A B C D E F G H I)
essai=${ESSAI:-}
[[ -n $essai ]] && log=run-essai.log
: >> "$log"

horodater() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$log" >&2; }
voulu() { [[ " ${sections[*]} " == *" $1 "* ]]; }

# Préchauffage : 10 s de jeu, 120 s pour l'assaut (foule formée).
pc() { [[ $1 == assaut ]] && echo 2400 || echo 200; }

# Configuration retenue (hypothèse de départ de la campagne).
RET=(--resolution 2 --regle majorite --periode-champ 20 --recalcul partiel --voisinage dense16 --reservation partout)

# Nom de fichier de résultats (préfixé « essai- » en mode essai).
f() { if [[ -n $essai ]]; then echo "essai-$1"; else echo "$1"; fi; }
out=$(f resultats.csv)
# mesurer <scénario> <tiles> <options...>
mesurer() {
  local sc=$1 n=$2
  shift 2
  local p
  p=$(pc "$sc")
  if [[ -n $essai ]]; then
    n=1; p=2; set -- "$@" --ticks 2
  fi
  "$bin" --scenario "$sc" --tiles "$n" --prechauffage "$p" "$@" >> "$out" 2>> "$log"
}

if [[ ! -s $out ]]; then "$bin" --entete > "$out"; fi

# ------------------------------------------------------------------ A. Bancs
if voulu A; then
  horodater "A. banc des champs"
  t=8; r=5; [[ -n $essai ]] && t=1 && r=1
  "$bin" --banc-champs --entete > "$(f resultats-champs.csv)"
  "$bin" --banc-champs --tiles "$t" --repetitions "$r" >> "$(f resultats-champs.csv)" 2>> "$log"
  horodater "A. banc des frontières"
  "$bin" --banc-frontieres --entete > "$(f resultats-frontieres.csv)"
  if [[ -z $essai ]]; then "$bin" --banc-frontieres >> "$(f resultats-frontieres.csv)" 2>> "$log"; fi
  horodater "A. banc des index de zombies"
  "$bin" --banc-voisinage --entete > "$(f resultats-voisinage.csv)"
  for sc in calme disperse assaut; do
    t=16; p=$(pc "$sc"); [[ -n $essai ]] && t=1 && p=2
    "$bin" --banc-voisinage --scenario "$sc" --tiles "$t" --prechauffage "$p" "${RET[@]}" >> "$(f resultats-voisinage.csv)" 2>> "$log"
  done
fi

# ---------------------------------------------------- B. Index des zombies
if voulu B; then
  for sc in calme disperse assaut; do
    horodater "B. index des zombies, $sc, 16 tiles"
    for v in dense32 dense16 triee creuse; do
      mesurer "$sc" 16 "${RET[@]}" --voisinage "$v" --chauffe-voisinage dense16
    done
    mesurer "$sc" 16 "${RET[@]}" --case-zombies 4
  done
  horodater "B. index des zombies, calme, 64 tiles"
  for v in dense32 dense16 triee creuse; do
    mesurer calme 64 "${RET[@]}" --voisinage "$v" --chauffe-voisinage dense16
  done
  mesurer calme 64 "${RET[@]}" --case-zombies 4
fi

# ------------------------------------------------------------------ C. Champs
if voulu C; then
  for sc in calme disperse assaut repli; do
    horodater "C. champs, $sc, 16 tiles"
    # Résolution (période retenue, étalé).
    for res in 1 2 4 5; do
      mesurer "$sc" 16 "${RET[@]}" --resolution "$res"
    done
    mesurer "$sc" 16 "${RET[@]}" --regle toute
    # Période, étalée ou d'un seul tenant.
    for per in 1 5 60; do
      mesurer "$sc" 16 "${RET[@]}" --periode-champ "$per"
    done
    mesurer "$sc" 16 "${RET[@]}" --periode-champ 5 --sans-etalement
    mesurer "$sc" 16 "${RET[@]}" --sans-etalement
    # Sans flow field (ligne droite, P-01 test 1).
    mesurer "$sc" 16 "${RET[@]}" --champ aucun
  done
  horodater "C. étalement, 1 tile"
  for sc in calme disperse assaut repli; do
    for per in 5 20; do
      mesurer "$sc" 1 "${RET[@]}" --periode-champ "$per"
      mesurer "$sc" 1 "${RET[@]}" --periode-champ "$per" --sans-etalement
    done
  done
fi

# ----------------------------------------------------------- D. Réservation
if voulu D; then
  horodater "D. réservation"
  for r in aucune contact partout; do
    for n in 1 16; do
      mesurer assaut "$n" "${RET[@]}" --reservation "$r"
      mesurer assaut "$n" "${RET[@]}" --reservation "$r" --barricade-initiale
      mesurer disperse "$n" "${RET[@]}" --reservation "$r"
    done
  done
fi

# ------------------------------------------------- E. Obstacle qui apparaît
if voulu E; then
  horodater "E. obstacle pendant l'assaut"
  for rc in partiel complet; do
    for n in 1 16; do
      mesurer assaut "$n" "${RET[@]}" --barricade --recalcul "$rc"
    done
  done
fi

# ------------------------------------------------- F. Montée en charge
if voulu F; then
  horodater "F. montée en charge : calme"
  for n in 1 2 4 8 16 24 32 48 64; do
    mesurer calme "$n" "${RET[@]}"
  done
  for sc in disperse assaut repli; do
    horodater "F. montée en charge : $sc"
    for n in 1 2 4 8 16 24 32; do
      mesurer "$sc" "$n" "${RET[@]}"
    done
  done
fi

# ------------------------------------------------- G. Référence P-02
if voulu G; then
  horodater "G. référence P-02 (vue 400 m)"
  outp=$(f resultats-p02.csv)
  "$p02" --entete > "$outp"
  ref=(--structure kdtree --rejet --relais --filtre-relais --veille --profonde --vue 400)
  for n in 1 16 32 64; do
    t=$n; p=200; x=(); [[ -n $essai ]] && t=1 && p=2 && x=(--ticks 2)
    "$p02" --scenario calme --tiles "$t" --prechauffage "$p" "${ref[@]}" "${x[@]}" >> "$outp" 2>> "$log"
  done
  for n in 1 16 32; do
    for sc in disperse assaut; do
      t=$n; p=$(pc "$sc"); x=(); [[ -n $essai ]] && t=1 && p=2 && x=(--ticks 2)
      "$p02" --scenario "$sc" --tiles "$t" --prechauffage "$p" "${ref[@]}" "${x[@]}" >> "$outp" 2>> "$log"
    done
  done
fi

# ----------------------------------------------------------------- H. Threads
if voulu H; then
  horodater "H. threads"
  for t in 1 2 4 8 16; do
    mesurer disperse 16 "${RET[@]}" --threads "$t"
  done
fi

# ----------------------------------------------------- I. Distance de vue
if voulu I; then
  horodater "I. vue de 200 m"
  for n in 16 64; do
    mesurer calme "$n" "${RET[@]}" --vue 200
  done
  mesurer disperse 16 "${RET[@]}" --vue 200
  mesurer assaut 16 "${RET[@]}" --vue 200
fi

horodater "Terminé : $out"
