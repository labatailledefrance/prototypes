#!/bin/bash
# P-09 — campagne de mesures. Environ 25 min.
# ESSAI=1 ./run.sh : valide toutes les commandes en deux minutes (500 paires,
# résultats dans essai/).
# Rien de lourd en parallèle pendant la campagne.
# Images de vérification seules : ./run.sh images
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release 2>&1 | tail -1
B=target/release/p09-chemin-routier
G=../../data/generated
MANCHE=(--tiles $G/p07-manche --etiquette manche --controle-km 31088.255
  --lieux "cherbourg=366500,6959500;saint-lo=401500,6898500;granville=362500,6869500;la-hague=343500,6968500;mont-saint-michel=367500,6846500;maupertus=377500,6959500;port-cherbourg=367500,6959500"
  --trajets "cherbourg>saint-lo,cherbourg>granville,la-hague>mont-saint-michel,maupertus>port-cherbourg")
# Paris : coordonnées approchées (à 100 m près) de l'Étoile, de la gare de
# Lyon, de Notre-Dame, des portes de Saint-Cloud et de Bagnolet.
PARIS=(--tiles $G/p07-paris --etiquette paris --controle-km 2789.902
  --lieux "etoile=648300,6864300;gare-de-lyon=653900,6861700;notre-dame=652300,6861900;porte-saint-cloud=644900,6859900;porte-bagnolet=657300,6863600"
  --trajets "etoile>gare-de-lyon,porte-saint-cloud>porte-bagnolet,notre-dame>etoile")

images() {
  $B --mode images "${MANCHE[@]}" --sortie $G/p09-manche > /dev/null
  $B --mode images "${PARIS[@]}" --sortie $G/p09-paris > /dev/null
  mkdir -p images
  cp $G/p09-manche/images/*.png $G/p09-paris/images/*.png images/
}
if [[ "${1:-}" == images ]]; then
  images; exit 0
fi

if [[ "${ESSAI:-0}" == 1 ]]; then
  O=essai; PFX=$G/p09-essai; REQ=(--paires 500 --banc 50 --convois 200); DET=(--paires 300)
else
  O=resultats; PFX=$G/p09; REQ=(--paires 10000 --banc 1000 --convois 1000); DET=(--paires 2000)
fi
mkdir -p $O
FICHIERS=(construction ponts ponts-distribution refus pretraitements requetes requetes-resume charge typiques modifs jeu)

# ------------------------------------------------------------ 1-5. Manche, puis 6. Paris
for z in manche paris; do
  if [[ $z == manche ]]; then A=("${MANCHE[@]}"); else A=("${PARIS[@]}"); fi
  D=$PFX-$z
  /usr/bin/time -f "rss_max_ko %M" $B --mode campagne "${A[@]}" "${REQ[@]}" --sortie $D > $D.log 2> $D.time || { cat $D.log; exit 1; }
  for f in "${FICHIERS[@]}"; do cp $D/$f.csv $O/$z-$f.csv; done
  cat $D.time >> $O/$z-construction.csv
  echo "$z : fait"
done

# ------------------------------------------------------------ 9. Fichier livré par la chaîne
for z in manche paris; do
  if [[ $z == manche ]]; then A=("${MANCHE[@]}"); else A=("${PARIS[@]}"); fi
  $B --mode fichier "${A[@]}" --sortie $PFX-$z > /dev/null 2>&1
  cp $PFX-$z/fichier.csv $O/$z-fichier.csv
done
echo "fichier : fait"

# ------------------------------------------------------------ 7. Déterminisme
F=$G/p09-det; rm -rf $F; mkdir -p $F
for z in manche paris; do
  if [[ $z == manche ]]; then A=("${MANCHE[@]}"); else A=("${PARIS[@]}"); fi
  for run in "16 direct" "1 direct" "16 melange" "16 inverse" "16 direct"; do
    set -- $run
    $B --mode determinisme "${A[@]}" "${DET[@]}" --threads $1 --ordre $2 --sortie $F > /dev/null 2>&1
  done
done
cp $F/determinisme.csv $O/determinisme.csv
echo "déterminisme : fait"

# ------------------------------------------------------------ Tests et micro-bancs
cargo test --release 2>&1 | grep -E "test result|FAILED|panicked" > $O/tests.txt || true
cargo test --release banc -- --ignored --nocapture 2> $O/banc.txt > /dev/null || true
grep "^banc" $O/banc.txt > $O/banc.tmp && mv $O/banc.tmp $O/banc.txt
echo "tests : fait"
