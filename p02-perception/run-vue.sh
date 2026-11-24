#!/usr/bin/env bash
# P-02, complément — distance de vue avec veille profonde au-delà de la vue
# (seuil = vue + 100 m). Configuration retenue. Écrit resultats-vue.csv.
# Durée : environ 10 min.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
bin=target/release/p02-perception
out=resultats-vue.csv
"$bin" --entete > "$out"
ret=(--structure kdtree --rejet --relais --filtre-relais --veille --profonde)
for vue in 200 400 800; do
  for n in 1 16; do
    "$bin" --scenario calme --tiles "$n" --prechauffage 200 --vue "$vue" "${ret[@]}" >> "$out"
    "$bin" --scenario disperse --tiles "$n" --prechauffage 200 --vue "$vue" "${ret[@]}" >> "$out"
    "$bin" --scenario assaut --tiles "$n" --prechauffage 2400 --vue "$vue" "${ret[@]}" >> "$out"
  done
done
echo "Résultats : $out"
