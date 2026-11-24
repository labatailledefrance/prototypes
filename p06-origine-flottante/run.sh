#!/bin/bash
# P-06 — campagne de mesures. Environ 20 min.
# ESSAI=1 ./run.sh : valide toutes les commandes en quelques minutes (durées
# réduites, résultats dans essai/).
# Rien de lourd en parallèle pendant la campagne. Les modes fenêtrés ouvrent une
# fenêtre de 1920 × 1080 sur la RTX 3060 (session Wayland).
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release 2>&1 | tail -1
B=target/release/p06-origine-flottante
export WGPU_ADAPTER_NAME="RTX 3060"
if [[ "${ESSAI:-0}" == 1 ]]; then
  D=essai; mkdir -p $D
  FR=300; FRZ=600; ACC_NAV=200000; CAP=8; NS=(10000); NC=(10000); FRC=100; SIM_N=100000; SIM_T=40; REP=1
else
  D=.; FR=3600; FRZ=7200; ACC_NAV=20000; CAP=60; NS=(10000 100000 1000000); NC=(10000 100000 1000000); FRC=600; SIM_N=1000000; SIM_T=200; REP=3
fi
LIEUX=("Brest" "Papeete" "Île Walpole")
STRATS=(naif-paris naif-ecef chaque-frame seuil:1000 seuil:10000 tiles big-space:1000)

# ------------------------------------------------------------ 1. Géodésie
$B --mode geo --entete > $D/resultats-geo.csv
# Validation des projections contre PROJ (cs2cs) : écart en mm.
{
  echo "lieu,epsg,ecart_proj_mm"
  $B --mode geo --proj | while IFS=$'\t' read -r nom epsg lat lon e n; do
    r=$(echo "$lat $lon" | cs2cs -f %.4f EPSG:4326 EPSG:$epsg 2>/dev/null | awk '{print $1, $2}')
    # La Polynésie (RGPF) passe par une transformation de datum : on compare
    # aussi la projection seule, sur l'ellipsoïde GRS80.
    case $epsg in 329?) z=$((epsg-3291)); r=$(echo "$lon $lat" | cs2cs -f %.4f +proj=longlat +ellps=GRS80 +to +proj=utm +zone=$z +south +ellps=GRS80 | awk '{print $1, $2}');; esac
    echo "$nom $epsg $e $n $r" | awk '{dx=$3-$5; dy=$4-$6; printf "%s,%s,%.4f\n",$1,$2,sqrt(dx*dx+dy*dy)*1000}'
  done
} > $D/resultats-proj.csv
echo "géodésie : fait"

# ------------------------------------------------------------ 2. Précision (relecture de la projection)
F=$D/resultats-precision.csv
$B --mode precision --entete --frames 1 --strategie chaque-frame | head -1 > $F
# Scénarios 1 et 2 : soldat qui marche, même instant de jeu (jour 20), 60 s.
for l in "${LIEUX[@]}"; do
  for s in "${STRATS[@]}"; do
    $B --mode precision --scenario soldat --lieu "$l" --strategie $s --frames $FR --etiquette soldat >> $F
  done
  # Distance caméra–origine imposée : pire cas d'un seuil.
  for dec in 100 1000 10000 100000 1000000; do
    $B --mode precision --scenario soldat --lieu "$l" --strategie decalage:$dec --frames $FR --etiquette decalage >> $F
  done
done
# Scénario 3 : navire Brest → Panama → Papeete → Nouméa, accéléré.
for s in naif-paris naif-ecef chaque-frame seuil:1000 big-space:1000; do
  $B --mode precision --scenario navire --acceleration $ACC_NAV --strategie $s --etiquette navire \
     --chrono $D/chrono-navire-${s%%:*}.csv >> $F 2> /dev/null
done
# Scénario 4 : zoom du soldat au globe et retour.
for l in Papeete "Île Walpole"; do
  for s in naif-paris chaque-frame seuil:1000 big-space:1000; do
    n=$(echo "$l" | sed 's/Î/I/; s/ //g')
    $B --mode precision --scenario zoom --lieu "$l" --strategie $s --frames $FRZ --etiquette zoom \
       --chrono $D/chrono-zoom-$n-${s%%:*}.csv >> $F
  done
done
# Scénario 5 : partie longue (jour 42, ×100) : positions de la simulation.
for s in chaque-frame seuil:1000; do
  $B --mode precision --scenario soldat --lieu "Île Walpole" --strategie $s --jour 42 --acceleration 100 --frames $FR --etiquette longue >> $F
done
echo "précision : fait"

# ------------------------------------------------------------ 3. Horloge des shaders
$B --mode temps --entete > $D/resultats-temps.csv 2> $D/resultats-temps.txt
echo "temps : fait"

# ------------------------------------------------------------ 4. Rendu réel : captures, profondeur, ombres
F=$D/resultats-captures.csv
first=1
for l in "${LIEUX[@]}"; do
  for s in naif-paris naif-ecef chaque-frame seuil:1000 big-space:1000; do
    if [[ $first == 1 ]]; then E=(--entete); first=0; else E=(); fi
    $B --mode capture --lieu "$l" --strategie $s --frames $CAP "${E[@]}" 2> /dev/null | grep -E "^(etiquette|,)" >> $F
  done
done
F=$D/resultats-profondeur.csv
$B --mode profondeur --strategie chaque-frame --entete 2> /dev/null | grep -E "^(etiquette|,)" > $F
$B --mode profondeur --strategie naif-paris 2> /dev/null | grep -E "^," >> $F
F=$D/resultats-ombres.csv
$B --mode ombres --lieu Papeete --entete 2> /dev/null | grep -E "^(etiquette|,)" > $F
$B --mode ombres --lieu Papeete --ombres --sortie $D/images 2> /dev/null | grep -E "^," >> $F
$B --mode ombres --lieu Brest --ombres 2> /dev/null | grep -E "^," >> $F
echo "rendu réel : fait"

# ------------------------------------------------------------ 5. Coût du recalage
F=$D/resultats-cout.csv
$B --mode cout --entete --n 10 --frames 1 --strategie seuil:1000 | head -1 > $F
for k in $(seq $REP); do
  for n in "${NS[@]}"; do
    for s in chaque-frame seuil:1000 tiles tiles-seuil:1000 big-space:1000; do
      $B --mode cout --n $n --frames $FRC --strategie $s --etiquette "r$k" >> $F
    done
  done
done
# Sans mobiles : effet du moindre enfant qui bouge sur la propagation.
for s in seuil:1000 tiles-seuil:1000 big-space:1000; do
  $B --mode cout --n ${NS[-1]} --frames $FRC --mobiles 0 --strategie $s --etiquette sans-mobiles >> $F
done
F=$D/resultats-cadence.csv
first=1
for n in "${NC[@]}"; do
  for s in chaque-frame seuil:1000 big-space:1000; do
    if [[ $first == 1 ]]; then E=(--entete); first=0; else E=(); fi
    $B --mode cadence --n $n --frames $FRC --strategie $s "${E[@]}" 2> /dev/null | grep -E "^(etiquette|,)" >> $F
  done
done
echo "coût : fait"

# ------------------------------------------------------------ 6. Simulation : déterminisme, coût, mémoire
F=$D/resultats-simu.csv
$B --mode simu --entete --n 10 --ticks 2 | head -1 > $F
for r in c b a; do
  for th in 1 4 16; do
    $B --mode simu --repr $r --n $SIM_N --ticks $SIM_T --threads $th --etiquette threads >> $F
  done
  $B --mode simu --repr $r --n $SIM_N --ticks $SIM_T --lot 64 --etiquette lots >> $F
  $B --mode simu --repr $r --n $SIM_N --ticks $SIM_T --lot 100000 --etiquette lots >> $F
  # Même départ local ailleurs dans le monde.
  for l in Papeete "Île Walpole" Nouméa; do
    $B --mode simu --repr $r --n $SIM_N --ticks $SIM_T --lieu "$l" --etiquette translation >> $F
  done
done
# Caméra ailleurs, rendu relatif à la caméra dans le même App : même somme.
for l in Brest Papeete "Île Walpole"; do
  $B --mode simu --repr c --n $SIM_N --ticks $SIM_T --rendu "$l" --etiquette camera >> $F
done
$B --mode derive --entete > $D/resultats-derive.csv
$B --mode sauvegarde --entete > $D/resultats-sauvegarde.csv
echo "simulation : fait"

# ------------------------------------------------------------ 7. Micro-bancs et tests
cargo test --release banc -- --ignored --nocapture 2>/dev/null | grep -E "^(marche|recalage)" > $D/banc.txt
cargo test --release 2>&1 | grep -E "test result" > $D/tests.txt
echo "bancs et tests : fait"
