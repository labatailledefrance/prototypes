#!/usr/bin/env bash
# P-00 — coût de la fragmentation en archétypes (Bevy, jauges groupées,
# détection de changement active), avec le découpage par défaut (lot ≥ 1)
# et avec une taille de lot minimale de 4 096. Écrit resultats-archetypes.csv.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
bin=target/release/p00-socle-ecs
sortie=resultats-archetypes.csv
"$bin" --entete > "$sortie"
for n in 20000 100000 1000000; do
  for k in 1 4 16 64 256 1024; do
    "$bin" --impl bevy --jauges groupees --mode mono --archetypes "$k" --entites "$n" >> "$sortie"
    for t in 4 16; do
      for lot in 1 4096; do
        "$bin" --impl bevy --jauges groupees --mode par --threads "$t" --archetypes "$k" --lot-min "$lot" --entites "$n" >> "$sortie"
      done
    done
  done
done
echo "Résultats : $sortie"
