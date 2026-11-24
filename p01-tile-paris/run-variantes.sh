#!/usr/bin/env bash
# P-01, test 2 — relais d'alerte et mise en veille, sur ville synthétique et
# sur emprises réelles. Écrit resultats-variantes.csv et
# resultats-variantes-chronologie.csv.
# Prérequis (ville réelle) : outils/extraire_tiles.py (voir README).
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
bin=target/release/p01-tile-paris
out=resultats-variantes.csv
# VILLES="reelle" ./run-variantes.sh : ne refait que ces villes et garde les
# autres lignes du fichier.
villes=${VILLES:-synthetique reelle}
if [[ -n "${VILLES:-}" && -f "$out" ]]; then
  garder=$(grep -v -i -E "^[^,]*,($(echo "$villes" | tr ' ' '|'))," "$out")
  echo "$garder" > "$out"
else
  "$bin" --entete > "$out"
fi
variantes=("" "--relais" "--relais --veille")
for ville in $villes; do
  for v in "${variantes[@]}"; do
    # shellcheck disable=SC2086
    for n in 1 2 4 8 16 24 32 48 64; do
      "$bin" --ville "$ville" --scenario calme --tiles "$n" --prechauffage 200 $v >> "$out"
    done
    for n in 1 2 4 8 16 24 32; do
      "$bin" --ville "$ville" --scenario disperse --tiles "$n" --prechauffage 200 $v >> "$out"
      # Assaut mesuré après 120 s de jeu, foule formée.
      "$bin" --ville "$ville" --scenario assaut --tiles "$n" --prechauffage 2400 $v >> "$out"
    done
  done
done
# Fraîcheur de l'alerte (portée de la vague contre délai de perte).
for f in 1 3 10; do
  "$bin" --ville reelle --scenario assaut --relais --fraicheur "$f" --prechauffage 2400 >> "$out"
done
# Chronologie d'un assaut sur tile réelle, avec et sans relais.
chrono=resultats-variantes-chronologie.csv
"$bin" --ville reelle --scenario assaut --chronologie --ticks 6000 --fenetre 300 > "$chrono"
"$bin" --ville reelle --scenario assaut --relais --chronologie --ticks 6000 --fenetre 300 | tail -n +2 >> "$chrono"
echo "Résultats : $out, $chrono"
