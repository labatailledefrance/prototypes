#!/usr/bin/env bash
# P-02 — campagne de mesure. Écrit :
#   resultats-banc.csv : banc mono-thread de toutes les structures ;
#   resultats.csv      : simulation complète (matrice, montée en charge,
#                        distance de vue, threads).
# Durée : environ 40 min sur la machine de référence. Ne rien lancer de
# lourd en parallèle.
# Prérequis : tiles réelles dans ../../data/generated/p01-tiles (voir
# ../p01-tile-paris/README.md, test 2).
set -euo pipefail
cd "$(dirname "$0")"
# `cargo test` ne reconstruit pas le binaire release : on le fait ici.
cargo build --release --quiet
bin=target/release/p02-perception
banc=resultats-banc.csv
out=resultats.csv
log=run.log
# SANS_BANC=1 ./run.sh : garde resultats-banc.csv.
: > "$log"

horodater() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$log" >&2; }

# Préchauffage : 10 s de jeu, 120 s pour l'assaut (foule formée).
pc() { [[ $1 == assaut ]] && echo 2400 || echo 200; }

# mesurer <scénario> <tiles> <options...>
mesurer() {
  local sc=$1 n=$2
  shift 2
  "$bin" --scenario "$sc" --tiles "$n" --prechauffage "$(pc "$sc")" "$@" >> "$out" 2>> "$log"
}

# ------------------------------------------------------------------ A. Banc
if [[ -z "${SANS_BANC:-}" ]]; then
horodater "A. banc des structures"
"$bin" --banc --entete > "$banc"
for sc in calme disperse assaut; do
  for v in "" "--relais --veille"; do
    # shellcheck disable=SC2086
    "$bin" --banc --scenario "$sc" --prechauffage "$(pc "$sc")" --repetitions 5 $v >> "$banc" 2>> "$log"
  done
done
fi

# --------------------------------------------------------------- B. Matrice
"$bin" --entete > "$out"
structures=("--structure grille --case 25" "--structure grille --case 50" "--structure anneaux --case 25" "--structure hierarchique --case 12.5" "--structure kdtree" "--structure kdtree --rejet")
variantes=("" "--relais --veille")
# Configuration retenue, et ses étapes.
ret="--structure kdtree --rejet --relais --filtre-relais --veille --profonde"
etapes=("--structure kdtree --rejet --veille" "--structure kdtree --rejet --relais --filtre-relais --veille" "$ret")
# Référence P-01 : grille de 25 m, relais et veille.
ref="--structure grille --case 25 --relais --veille"
for n in 1 16; do
  for sc in calme disperse assaut; do
    horodater "B. matrice, $sc, $n tiles"
    for v in "${variantes[@]}"; do
      for s in "${structures[@]}"; do
        # shellcheck disable=SC2086
        mesurer "$sc" "$n" $s $v
      done
    done
    for e in "${etapes[@]}"; do
      # shellcheck disable=SC2086
      mesurer "$sc" "$n" $e
    done
    # Humains immobiles (comme P-01) : coût de la reconstruction de l'index.
    # shellcheck disable=SC2086
    mesurer "$sc" "$n" $ref --immobiles
    # shellcheck disable=SC2086
    mesurer "$sc" "$n" $ret --immobiles
  done
done

# ------------------------------------------------------ C. Montée en charge
for cfg in "$ref" "$ret"; do
  horodater "C. montée en charge : $cfg"
  for n in 2 4 8 24 32 48 64; do
    # shellcheck disable=SC2086
    mesurer calme "$n" $cfg
  done
  for n in 2 4 8 24 32; do
    # shellcheck disable=SC2086
    mesurer disperse "$n" $cfg
    # shellcheck disable=SC2086
    mesurer assaut "$n" $cfg
  done
done

# ------------------------------------------------------ D. Distance de vue
horodater "D. distance de vue"
for sc in disperse assaut; do
  for vue in 100 400; do
    mesurer "$sc" 16 --structure grille --case 25 --vue "$vue"
    mesurer "$sc" 16 --structure kdtree --rejet --vue "$vue"
  done
done

# ----------------------------------------------------------------- E. Threads
horodater "E. threads"
for t in 1 2 4 8; do
  # shellcheck disable=SC2086
  mesurer disperse 16 $ret --threads "$t"
done

horodater "Terminé : $banc, $out"
