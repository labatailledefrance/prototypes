#!/bin/bash
# P-07 — campagne de mesures. Environ 1 h.
# ESSAI=1 ./run.sh : valide toutes les commandes en deux minutes (six tiles,
# relief 5 m natif, résultats dans essai/).
# Rien de lourd en parallèle pendant la campagne.
# Images de vérification seules : ./run.sh images
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release 2>&1 | tail -1
B=target/release/p07-chaine-manche
R=../../data
S=$R/sources
G=$R/generated
GPKG50=$S/bdtopo-d050-2026-09-15/D050.gpkg
GPKG75=$S/bdtopo-d075-2026-09-15/BDT_3-5_GPKG_LAMB93_D075-ED2026-09-15.gpkg
RGE1=$S/rgealti-d050-1m-2022-12-21/RGEALTI_2-0_1M_ASC_LAMB93-IGN69_D050_2022-12-21.7z
RGE5=$S/rgealti-d050-5m-2022-12-21/RGEALTI_2-0_5M_ASC_LAMB93-IGN69_D050_2022-12-21.7z
INSEE=$S/insee-filosofi2021-carreaux-2026-11-16/carreaux-200m-met-3035-2021.parquet
CAR1K=$S/insee-filosofi2021-carreaux-2026-11-16/carreaux_1km_met.csv
# Tiles typiques (centre de Cherbourg, Saint-Lô, bocage, côte, baie du
# Mont-Saint-Michel, La Hague, Flamanville, Granville, pleine mer).
TYPIQUES=x366_y6958:cherbourg,x401_y6898:saint-lo,x397_y6878:bocage,x352_y6929:cote,x367_y6846:mont-saint-michel,x347_y6964:la-hague,x346_y6948:flamanville,x362_y6869:granville,x330_y6930:mer
ESSAI_TILES=x366_y6958,x401_y6898,x397_y6878,x352_y6929,x367_y6846,x347_y6964

images() {
  local D=$1
  $B --mode images --sortie $D --tiles $TYPIQUES > /dev/null
  mkdir -p images
  cp $D/images/*.png images/
}
if [[ "${1:-}" == images ]]; then
  images $G/p07-manche; exit 0
fi

if [[ "${ESSAI:-0}" == 1 ]]; then
  O=essai; mkdir -p $O; PFX=$G/p07-essai
  RELIEF=(--rge5m $RGE5 --relief-pas 5); STATS=(--rge5m $RGE5 --relief-pas 5 --essai $ESSAI_TILES)
  SUB=(--essai $ESSAI_TILES); THREADS=(16 1)
else
  O=.; PFX=$G/p07
  RELIEF=(--rge1m $RGE1 --rge5m $RGE5); STATS=(--rge1m $RGE1 --rge5m $RGE5 --stats-relief)
  SUB=(); THREADS=(16 16 8 4 1)
fi
COMMUN=(--gpkg $GPKG50 --insee200 $INSEE)
ETAPES=$O/resultats-etapes.csv; CTRL=$O/resultats-controles.txt
echo "etiquette,etape,s,rss_mo,rss_max_mo" > $ETAPES; : > $CTRL
garder() { # dossier de sortie → résultats
  tail -n +2 $1/etapes.csv >> $ETAPES
  { cat $1/controles.txt; echo; } >> $CTRL
}

# ------------------------------------------------------------ 0. Projections
$B --mode verif-geo > $O/resultats-geo.csv

# ------------------------------------------------------------ 1. BD TOPO : décompression (place temporaire)
T=$G/p07-tmp; rm -rf $T; mkdir -p $T
/usr/bin/time -f "bdtopo_7z_s %e\nbdtopo_7z_rss_ko %M" 7z x -y -o$T $S/bdtopo-d050-2026-09-15/*.7z > /dev/null 2> $O/resultats-decompression.txt
du -sb $T | awk '{print "bdtopo_gpkg_octets", $1}' >> $O/resultats-decompression.txt
rm -rf $T
echo "décompression : fait"

# ------------------------------------------------------------ 2. Manche, de bout en bout (relief 2 m, 10 cm)
D=$PFX-manche
$B --mode chaine "${COMMUN[@]}" "${RELIEF[@]}" "${SUB[@]}" --sortie $D --etiquette manche > /dev/null 2>&1
garder $D
du -sb $D/paquets | awk '{print "paquets_octets", $1}' >> $CTRL
cp $D/tiles.csv $O/resultats-tiles-manche.csv
cp $D/communes.csv $O/resultats-communes.csv
cp $D/lieux.csv $O/resultats-lieux.csv
$B --mode charger --sortie $D --etiquette manche 2> /dev/null
cp $D/charge.csv $O/resultats-charge.csv
cp $D/charge-resume.csv $O/resultats-charge-resume.csv
cp $D/niveaux.csv $O/resultats-niveaux.csv
tail -3 $D/controles.txt >> $CTRL
images $D
echo "manche : fait"

# ------------------------------------------------------------ 3. Déterminisme : mêmes octets
F=$O/resultats-determinisme.csv
echo "execution,threads,empreinte,total_s,rss_max_mo" > $F
k=0
for th in "${THREADS[@]}"; do
  k=$((k + 1)); D2=$PFX-manche-$k
  $B --mode chaine "${COMMUN[@]}" "${RELIEF[@]}" "${SUB[@]}" --threads $th --sortie $D2 --etiquette manche-t$th-$k > /dev/null 2>&1
  garder $D2
  echo "$k,$th,$(grep '^empreinte' $D2/controles.txt | cut -d' ' -f2),$(grep '^total_s' $D2/controles.txt | cut -d' ' -f2),$(grep '^rss_max' $D2/controles.txt | cut -d' ' -f2)" >> $F
  # Mêmes octets fichier par fichier.
  if diff -rq $PFX-manche/paquets $D2/paquets > /dev/null; then echo "  $th threads : paquets identiques"; else echo "  $th threads : DIFFÉRENTS"; fi
  rm -rf $D2
done
echo "déterminisme : fait"

# ------------------------------------------------------------ 4. Variantes du relief (1, 2, 5 m ; 1 et 10 cm ; 5 m natif)
D=$PFX-stats
$B --mode chaine "${COMMUN[@]}" "${STATS[@]}" --sortie $D --etiquette stats-relief > /dev/null 2>&1
garder $D
[[ -f $D/relief.csv ]] && cp $D/relief.csv $O/resultats-relief.csv
for v in 1 5; do
  [[ "${ESSAI:-0}" == 1 ]] && continue
  D=$PFX-manche-r$v
  $B --mode chaine "${COMMUN[@]}" "${RELIEF[@]}" --relief-pas $v --sortie $D --etiquette relief-${v}m > /dev/null 2>&1
  garder $D
  du -sb $D/paquets | awk -v v=$v '{print "paquets_octets_relief_" v "m", $1}' >> $CTRL
  rm -rf $D
done
echo "relief : fait"

# ------------------------------------------------------------ 5. Sans les murs (effet sur la zone)
D=$PFX-sans-murs
$B --mode chaine "${COMMUN[@]}" --rge5m $RGE5 --relief-pas 5 "${SUB[@]}" --murs false --sortie $D --etiquette sans-murs > /dev/null 2>&1
garder $D; rm -rf $D
echo "murs : fait"

# ------------------------------------------------------------ 6. Paris (même chaîne, sans relief)
D=$PFX-paris
if [[ "${ESSAI:-0}" == 1 ]]; then PS=(--essai x651_y6862,x652_y6862); else PS=(); fi
$B --mode chaine --gpkg $GPKG75 --insee200 $INSEE --dept 75 --sortie $D "${PS[@]}" --etiquette paris > /dev/null 2>&1
garder $D
cp $D/tiles.csv $O/resultats-tiles-paris.csv
$B --mode charger --sortie $D --etiquette paris 2> /dev/null
tail -n +2 $D/charge-resume.csv >> $O/resultats-charge-resume.csv
tail -n +2 $D/charge.csv >> $O/resultats-charge.csv
$B --mode images --sortie $D --tiles x651_y6862:paris-dense,x648_y6858:paris-sud > /dev/null
cp $D/images/paris-*.png images/
echo "paris : fait"

# ------------------------------------------------------------ 7. Extrapolation
python3 -I outils/extrapoler.py --manche $O/resultats-tiles-manche.csv --etapes $PFX-manche/etapes.csv \
  --paris $O/resultats-tiles-paris.csv --carreaux $CAR1K --sources tailles-sources.csv \
  > $O/resultats-extrapolation.csv 2> $O/resultats-extrapolation.txt
echo "extrapolation : fait"

# ------------------------------------------------------------ 8. Tests et micro-bancs
cargo test --release 2>&1 | grep -E "test result|FAILED|panicked" > $O/tests.txt || true
cargo test --release banc -- --ignored --nocapture 2> /dev/null | grep "banc " > $O/banc.txt || true
echo "tests : fait"
