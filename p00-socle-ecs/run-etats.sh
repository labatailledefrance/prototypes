#!/usr/bin/env bash
# P-00 — test 3 : état fréquent (« en feu », 5 % des entités) porté par un
# composant marqueur ou par un champ. Écrit resultats-etats.csv.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
bin=target/release/p00-socle-ecs
sortie=resultats-etats.csv
"$bin" --etat champ --entete > "$sortie"
for n in 20000 1000000; do
  for c in 0 0.0001 0.001 0.01 0.1; do
    for etat in marqueur champ; do
      "$bin" --etat "$etat" --changements "$c" --threads 16 --entites "$n" >> "$sortie"
    done
  done
done
echo "Résultats : $sortie"
