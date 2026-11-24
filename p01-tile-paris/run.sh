#!/usr/bin/env bash
# P-01 — matrice des mesures. Écrit resultats.csv et resultats-chronologie.csv.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
bin=target/release/p01-tile-paris
out=resultats.csv
"$bin" --entete > "$out"
m() { "$bin" "$@" >> "$out"; }

# 1. Montée en charge : nombre de tiles actives simultanées.
#    Assaut : mesuré après 180 s de jeu (3 600 ticks), quand la foule est formée.
for n in 1 2 4 8 16 24 32 64; do
  m --scenario calme    --tiles "$n" --prechauffage 200
  m --scenario disperse --tiles "$n" --prechauffage 200
  m --scenario assaut   --tiles "$n" --prechauffage 3600
done

# 2. Taille de lot minimale (F-02).
for n in 1 16; do
  for lot in 64 256 1024 4096 16384; do
    m --scenario disperse --tiles "$n" --lot-min "$lot" --prechauffage 200
  done
done

# 3. Leviers de perception : période, distance de vue, lignes de vue tracées.
for n in 1 16; do
  for p in 1 2 5 10 20; do
    m --scenario disperse --tiles "$n" --periode "$p" --prechauffage 200
  done
done
for v in 50 100 200 400; do
  m --scenario disperse --tiles 16 --vue "$v" --prechauffage 200
done
for r in 0 1 4 8; do
  m --scenario disperse --tiles 16 --rayons "$r" --prechauffage 200
done

# 4. Threads (même somme de contrôle attendue sur chaque ligne).
for t in 1 2 4 8 16; do
  m --scenario disperse --tiles 16 --threads "$t" --prechauffage 200
done

# 5. Chronologie d'un assaut sur une tile (5 minutes de jeu).
"$bin" --scenario assaut --chronologie --ticks 6000 --fenetre 200 > resultats-chronologie.csv
echo "Résultats : $out, resultats-chronologie.csv"
