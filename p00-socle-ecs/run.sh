#!/usr/bin/env bash
# P-00 — matrice complète des mesures. Écrit resultats.csv.
# Usage : ./run.sh            (compilation portable)
#         NATIVE=1 ./run.sh   (target-cpu=native, écrit resultats-native.csv)
set -euo pipefail
cd "$(dirname "$0")"

sortie=resultats.csv
if [[ "${NATIVE:-0}" == 1 ]]; then
  export RUSTFLAGS="-C target-cpu=native"
  export CARGO_TARGET_DIR=target-native
  sortie=resultats-native.csv
fi
cargo build --release --quiet
bin="${CARGO_TARGET_DIR:-target}/release/p00-socle-ecs"

"$bin" --entete > "$sortie"
for n in 100000 1000000 10000000; do
  ticks=200; [[ $n -ge 10000000 ]] && ticks=100
  for variante in "brut" "bevy" "bevy --sans-detection"; do
    for jauges in separees groupees; do
      # shellcheck disable=SC2086
      "$bin" --impl $variante --jauges "$jauges" --mode mono --entites "$n" --ticks "$ticks" >> "$sortie"
      for t in 1 2 4 8 16; do
        # shellcheck disable=SC2086
        "$bin" --impl $variante --jauges "$jauges" --mode par --threads "$t" --entites "$n" --ticks "$ticks" >> "$sortie"
      done
    done
  done
done
echo "Résultats : $sortie"
