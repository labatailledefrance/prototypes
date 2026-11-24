#!/bin/bash
# P-08 — campagne de mesures. Environ 1 h 30.
# ESSAI=1 ./run.sh : valide toutes les commandes en quelques minutes (quelques
# tiles, frames réduites, résultats dans essai/).
# Captures de vérification seules : ./run.sh captures
# Rien de lourd en parallèle pendant la campagne. Le rendu est hors écran, sur
# la RTX 3060 (pas de fenêtre).
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release 2>&1 | tail -1
B=target/release/p08-generation-batiments
export WGPU_ADAPTER_NAME="RTX 3060"
R=../../data
S=$R/sources
G=$R/generated
M=(--sortie $G/p07-manche --gpkg $S/bdtopo-d050-2026-09-15/D050.gpkg)
P=(--sortie $G/p07-paris --gpkg $S/bdtopo-d075-2026-09-15/BDT_3-5_GPKG_LAMB93_D075-ED2026-09-15.gpkg)
# Tiles typiques (P-07) : centre de Cherbourg, Saint-Lô, bocage, La Hague
# (usine), Mont-Saint-Michel ; centre de Paris.
TYP_M=x366_y6958,x401_y6898,x397_y6878,x347_y6964,x367_y6846
TYP_P=x651_y6862
NOMS=(x366_y6958:cherbourg x401_y6898:saint-lo x397_y6878:bocage x347_y6964:la-hague x367_y6846:mont-saint-michel)

captures() {
  local O=$1 F=$2
  mkdir -p $O/images
  for tn in "${NOMS[@]}"; do
    t=${tn%%:*}; n=${tn##*:}
    for v in iso loin; do
      lod=iso; [[ $v == loin ]] && lod=loin
      $B --mode capture "${M[@]}" --tiles $t --cote 3 --vue $v --lod $lod --res $O/images --etiquette $n > /dev/null 2>&1
    done
  done
  for v in iso loin; do
    lod=iso; [[ $v == loin ]] && lod=loin
    $B --mode capture "${P[@]}" --tiles $TYP_P --cote 3 --vue $v --lod $lod --res $O/images --etiquette paris-centre > /dev/null 2>&1
  done
  # Destruction : intact, endommagé, effondré (vue isométrique).
  $B --mode rendu-destruction "${P[@]}" --tiles $TYP_P --cote 1 --organisation tuile --res $O/images --etiquette destruction-paris > /dev/null 2>&1
  # Portes (vue de dessus) : P-08 en rouge, règle de P-05 en bleu.
  $B --mode images "${M[@]}" --tiles $TYP_M --res $O/images > /dev/null 2>&1
  $B --mode images "${P[@]}" --tiles $TYP_P --res $O/images > /dev/null 2>&1
  echo "captures : fait ($O/images)"
}
if [[ "${1:-}" == captures ]]; then
  captures . 0; exit 0
fi

if [[ "${ESSAI:-0}" == 1 ]]; then
  O=essai; mkdir -p $O
  TM=(--tiles $TYP_M); TP=(--tiles $TYP_P,x652_y6862,x650_y6861)
  FR=120; COTES=(1 3); REP=1
else
  O=.; TM=(); TP=()
  FR=600; COTES=(1 3 7); REP=2
fi
RES=$O/resultats; mkdir -p $RES

# ------------------------------------------------------------ 1. Génération et contrôles, toutes les tiles
$B --mode stats "${M[@]}" "${TM[@]}" --res $RES --etiquette manche 2> /dev/null
$B --mode stats "${P[@]}" "${TP[@]}" --res $RES --etiquette paris 2> /dev/null
python3 -I outils/extrapoler.py --manche $RES/tiles-manche.csv --paris $RES/tiles-paris.csv \
  --p07 ../p07-chaine-manche/resultats-extrapolation.csv > $RES/extrapolation.csv 2> $RES/extrapolation.txt
echo "stats : fait"

# ------------------------------------------------------------ 2. Débit (1, 8, 16 threads)
$B --mode debit "${M[@]}" "${TM[@]}" --liste-threads 1,8,16 --res $RES --etiquette manche 2> /dev/null
$B --mode debit "${P[@]}" "${TP[@]}" --liste-threads 1,8,16 --res $RES --etiquette paris 2> /dev/null
echo "débit : fait"

# ------------------------------------------------------------ 3. Déterminisme : deux exécutions, threads, ordres
for k in 1 2; do
  $B --mode determinisme "${P[@]}" "${TP[@]}" --res $RES --etiquette paris-$k 2> /dev/null
  $B --mode determinisme "${M[@]}" --tiles $TYP_M --res $RES --etiquette manche-$k 2> /dev/null
done
for e in paris manche; do
  if diff <(cut -d, -f2- $RES/determinisme-$e-1.csv) <(cut -d, -f2- $RES/determinisme-$e-2.csv) > /dev/null; then
    echo "  $e : deux exécutions identiques"; else echo "  $e : DIFFÉRENTES"; fi
done
echo "déterminisme : fait"

# ------------------------------------------------------------ 4. Génération étalée (3 ms par tick, P-04)
$B --mode etale "${M[@]}" --tiles $TYP_M --res $RES --etiquette manche 2> /dev/null
$B --mode etale "${P[@]}" --tiles $TYP_P --res $RES --etiquette paris 2> /dev/null
echo "étalement : fait"

# ------------------------------------------------------------ 5. Destruction (CPU) et sauvegarde des différences
$B --mode destruction "${P[@]}" --tiles $TYP_P --res $RES --etiquette paris 2> /dev/null
$B --mode destruction "${M[@]}" --tiles x366_y6958 --res $RES --etiquette cherbourg 2> /dev/null
echo "destruction : fait"

# ------------------------------------------------------------ 6. Repli de la hauteur, format, monuments
for e in manche paris; do
  if [[ $e == manche ]]; then A=("${M[@]}" "${TM[@]}"); else A=("${P[@]}" "${TP[@]}"); fi
  $B --mode repli "${A[@]}" --res $RES --etiquette $e 2> /dev/null
  $B --mode format "${A[@]}" --res $RES --etiquette $e 2> /dev/null
  $B --mode monuments "${A[@]}" --res $RES --etiquette $e 2> /dev/null
done
echo "repli, format, monuments : fait"

# ------------------------------------------------------------ 7. Rendu : temps de frame et mémoire GPU (Paris, hors écran)
F=$RES/cadence.csv
$B --mode cadence "${P[@]}" --tiles $TYP_P --cote 0 --frames $FR --entete --etiquette sans-tile 2> /dev/null > $F
for c in "${COTES[@]}"; do
  for org in tuile morceaux batiment; do
    for r in $(seq $REP); do
      $B --mode cadence "${P[@]}" --tiles $TYP_P --cote $c --organisation $org --frames $FR --etiquette r$r 2> /dev/null >> $F
    done
  done
  $B --mode cadence "${P[@]}" --tiles $TYP_P --cote $c --organisation morceaux --compresse true --frames $FR --etiquette compresse 2> /dev/null >> $F
  $B --mode cadence "${P[@]}" --tiles $TYP_P --cote $c --organisation morceaux --ombres false --frames $FR --etiquette sans-ombres 2> /dev/null >> $F
  $B --mode cadence "${P[@]}" --tiles $TYP_P --cote $c --organisation morceaux --lod loin --frames $FR --etiquette loin 2> /dev/null >> $F
done
echo "cadence : fait"

# ------------------------------------------------------------ 8. Survol (streaming, génération en tâche de fond)
F=$RES/survol.csv; first=1
vol() { # etiquette, jeu (M|P), trajet, vitesse, altitude, rayon, lod, organisation, prégénéré, octets par frame
  local E=(); [[ $first == 1 ]] && E=(--entete) && first=0
  local J; if [[ $2 == M ]]; then J=("${M[@]}"); else J=("${P[@]}"); fi
  $B --mode survol "${J[@]}" --trajet $3 --vitesse $4 --altitude $5 --rayon $6 --lod $7 --organisation $8 --pregenere $9 --octets-par-frame ${10} \
     --res $RES --etiquette $1 "${E[@]}" 2> /dev/null >> $F
}
if [[ "${ESSAI:-0}" == 1 ]]; then
  PARIS_O=649.5,6862.5,652.5,6862.5; CHERB_S=366.5,6958.5,366.5,6955.5; CHERB_STLO=366.5,6958.5,371.5,6950.5
else
  PARIS_O=645.5,6862.5,657.5,6862.5; CHERB_S=366.5,6958.5,366.5,6940.5; CHERB_STLO=366.5,6958.5,401.5,6898.5
fi
# Organisation retenue : morceaux de 250 m, envoi étalé à 2 Mo par frame.
# Témoins : génération faite avant le vol ; tile fusionnée envoyée d'un bloc ;
# une entité par bâtiment.
for pg in false true; do
  vol paris-basse-$pg P $PARIS_O 50 250 1500 iso morceaux $pg 2000000
  vol paris-haute-$pg P $PARIS_O 400 3000 6000 loin morceaux $pg 2000000
  vol manche-basse-$pg M $CHERB_S 200 300 1500 iso morceaux $pg 2000000
  vol manche-haute-$pg M $CHERB_STLO 1000 5000 8000 loin morceaux $pg 2000000
done
vol paris-basse-tuile-bloc P $PARIS_O 50 250 1500 iso tuile false 0
vol paris-basse-tuile-file P $PARIS_O 50 250 1500 iso tuile false 2000000
vol paris-basse-batiment P $PARIS_O 50 250 1500 iso batiment false 2000000
vol paris-basse-rapide P $PARIS_O 200 250 1500 iso morceaux false 2000000
echo "survol : fait"

# ------------------------------------------------------------ 9. Destruction rendue (mise à jour d'un bâtiment)
F=$RES/rendu-destruction.csv; first=1
for org in tuile morceaux batiment; do
  E=(); [[ $first == 1 ]] && E=(--entete) && first=0
  $B --mode rendu-destruction "${P[@]}" --tiles $TYP_P --cote 3 --organisation $org --res $RES --etiquette $org "${E[@]}" 2> /dev/null >> $F
done
rm -f $RES/tuile-*.png $RES/morceaux-*.png $RES/batiment-*.png
echo "destruction rendue : fait"

# ------------------------------------------------------------ 10. Captures, tests, micro-bancs
captures $O 1
cargo test --release 2>&1 | grep -E "test result|FAILED|panicked" > $RES/tests.txt || true
cargo test --release banc -- --ignored --nocapture 2> /dev/null | grep "banc " > $RES/banc.txt || true
echo "tests : fait"
