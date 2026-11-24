#!/usr/bin/env bash
# P-03 — mesures refaites (valeurs suspectes de run.sh) ; ajoutées à resultats-verif.csv.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
bin=target/release/p03-flow-fields
RET=(--resolution 2 --regle majorite --periode-champ 20 --recalcul partiel --voisinage dense16 --reservation partout)
out=resultats-verif.csv
"$bin" --entete > "$out"
for k in 1 2 3; do
  for n in 16 24 32; do "$bin" --scenario repli --tiles $n "${RET[@]}" >> "$out" 2>/dev/null; done
  "$bin" --scenario disperse --tiles 16 "${RET[@]}" >> "$out" 2>/dev/null
done
