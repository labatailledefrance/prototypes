#!/bin/bash
# P-06 — vérification à l'œil. Lance une scène à l'écran (RTX 3060, temps réel).
#
#   ./voir.sh            liste des scènes
#   ./voir.sh <scène>    lance une scène ; Échap pour passer à la suivante
#   ./voir.sh tout       enchaîne toutes les scènes
#
# Touches : N stratégie suivante (témoin origine à Paris, témoin origine au
# centre de la Terre, recalage à chaque frame, recalage au-delà de 1 km) ;
# T horloge des shaders (bassin rayé : phase calculée sur les ticks, ou f32
# absolue) ; flèches haut/bas zoom ; + / − accélération ; espace pause.
# Le bandeau affiche l'écart mesuré (relecture de la projection) en pixels.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release 2>&1 | tail -1
B=target/release/p06-origine-flottante
export WGPU_ADAPTER_NAME="RTX 3060"

declare -A SCENES=(
  [brest]="--scenario soldat --lieu Brest --strategie chaque-frame"
  [tahiti]="--scenario soldat --lieu Papeete --strategie chaque-frame"
  [walpole]="--scenario soldat --lieu Île_Walpole --strategie chaque-frame"
  [temoin-brest]="--scenario soldat --lieu Brest --strategie naif-paris"
  [temoin-tahiti]="--scenario soldat --lieu Papeete --strategie naif-paris"
  [temoin-walpole]="--scenario soldat --lieu Île_Walpole --strategie naif-paris"
  [big-space]="--scenario soldat --lieu Île_Walpole --strategie big-space:1000"
  [navire]="--scenario navire --strategie chaque-frame --acceleration 2000"
  [zoom]="--scenario zoom --lieu Papeete --strategie chaque-frame --frames 3600"
  [horloge]="--scenario soldat --lieu Papeete --strategie chaque-frame --jour 42 --acceleration 100"
)
ORDRE=(brest tahiti walpole temoin-brest temoin-tahiti temoin-walpole big-space navire zoom horloge)

lancer() {
  local args=${SCENES[$1]}
  echo "== $1 : $args"
  # Les espaces des noms de lieux sont codés par « _ ».
  eval "set -- $args"
  local a=()
  for x in "$@"; do a+=("${x//_/ }"); done
  $B --mode voir "${a[@]}" 2> >(grep -E "panicked|ERROR" >&2) || true
}

case "${1:-}" in
  "") echo "Scènes : ${ORDRE[*]} (ou « tout »)";;
  tout) for s in "${ORDRE[@]}"; do lancer "$s"; done;;
  *) [[ -n "${SCENES[$1]:-}" ]] || { echo "scène inconnue : $1"; exit 1; }; lancer "$1";;
esac
