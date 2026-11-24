#!/bin/bash
# P-05 — campagne de mesures. Environ 1 h 15.
# ESSAI=1 ./run.sh : valide toutes les commandes en quelques secondes (durées
# réduites, résultats dans essai/).
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release 2>&1 | tail -1
B=target/release/p05-civils
if [[ "${ESSAI:-0}" == 1 ]]; then
  D=essai; mkdir -p $D
  CALME=(--chauffe 0 --ticks 20); SURVOL=(--duree 1); ASSAUT=(--duree 1); HIVER=(--jours 1 --ticks-par-jour 200)
  SAUVE=(--ticks 20); NS=(100000); N3=100000; REP=1
else
  D=.; CALME=(--chauffe 400 --ticks 2400); SURVOL=(--duree 600); ASSAUT=(--duree 1800); HIVER=(--jours 7)
  SAUVE=(--ticks 72000); NS=(100000 1000000 3000000); N3=3000000; REP=3
fi

# ------------------------------------------------------------ 1. Colonie calme
F=$D/resultats-calme.csv
$B --mode calme --entete > $F
for n in "${NS[@]}"; do
  for p in 20 200; do
    for r in a1 a2 b c; do
      $B --mode calme --repr $r --colons $n --periode $p --heure 6.5 "${CALME[@]}" --etiquette echelle >> $F
    done
  done
done
# Fréquences plus basses et heures de la journée (1 million).
for p in 600 1200; do
  for r in a2 b; do
    $B --mode calme --repr $r --colons 1000000 --periode $p --heure 6.5 "${CALME[@]}" --etiquette periode >> $F
    $B --mode calme --repr $r --colons $N3 --periode $p --heure 6.5 "${CALME[@]}" --etiquette periode >> $F
  done
done
for h in 3 12 18; do
  for r in a2 b; do
    $B --mode calme --repr $r --colons 1000000 --periode 200 --heure $h "${CALME[@]}" --etiquette heure >> $F
  done
done
# Threads et lots : même somme de contrôle.
for th in 1 4 8 16; do
  for p in 20 200; do
    $B --mode calme --repr b --colons 1000000 --periode $p --threads $th --heure 6.5 "${CALME[@]}" --etiquette threads >> $F
  done
done
for lm in 64 1024 100000; do
  $B --mode calme --repr b --colons 1000000 --periode 200 --lot-min $lm --heure 6.5 "${CALME[@]}" --etiquette lots >> $F
done
# Répétitions (bruit).
for k in $(seq $REP); do
  for r in a2 b; do
    $B --mode calme --repr $r --colons 1000000 --periode 200 --heure 6.5 "${CALME[@]}" --etiquette repetition >> $F
  done
done
echo "calme : fait"

# ------------------------------------------------------------ 2. Survol
F=$D/resultats-survol.csv
$B --mode survol --entete > $F
for v in 10 50 200; do
  for dl in 0 10 30 60; do
    $B --mode survol --heure 7 --vitesse-camera $v --delai $dl "${SURVOL[@]}" --etiquette survol >> $F
  done
done
# Équivalence : sans caméra, caméra sans marche réelle (même somme).
$B --mode survol --heure 7 --sans-camera --sans-marche "${SURVOL[@]}" --etiquette equivalence >> $F
$B --mode survol --heure 7 --sans-marche "${SURVOL[@]}" --etiquette equivalence >> $F
# Taille des lots d'individualisation (caméra rapide).
for li in 500 10000; do
  $B --mode survol --heure 7 --vitesse-camera 200 --lot-ind $li "${SURVOL[@]}" --etiquette lot >> $F
done
# Budgets de ×10 : lots et chemins divisés par 4.
$B --mode survol --heure 7 --vitesse-camera 50 --lot-ind 500 --budget-expansions 5000 "${SURVOL[@]}" --etiquette x10 >> $F
# Déclencheur « colon qui sort travailler » : chantiers toujours individualisés.
$B --mode survol --heure 10 --sans-camera --chantiers-individuels "${SURVOL[@]}" --etiquette chantiers >> $F
$B --mode survol --heure 10 --sans-camera "${SURVOL[@]}" --etiquette chantiers >> $F
echo "survol : fait"

# ------------------------------------------------------------ 3. Assaut
F=$D/resultats-assaut.csv
$B --mode assaut --entete > $F
for ar in 0 0.05 0.2; do
  $B --mode assaut --colons 30000 --heure 23 --armes $ar "${ASSAUT[@]}" --chrono $D/chrono-assaut-armes$ar.csv --etiquette armes >> $F
done
$B --mode assaut --colons 30000 --heure 23 --armes 0.05 --points-occupants "${ASSAUT[@]}" --chrono $D/chrono-assaut-occupants.csv --etiquette occupants >> $F
$B --mode assaut --colons 30000 --heure 23 --armes 0.05 --densite 0.25 "${ASSAUT[@]}" --chrono $D/chrono-assaut-banlieue.csv --etiquette banlieue >> $F
# Déterminisme : 5 min de jeu, 16 à 1 thread.
for th in 16 8 4 1; do
  if [[ "${ESSAI:-0}" == 1 ]]; then DU=(--duree 1); else DU=(--duree 300); fi
  $B --mode assaut --colons 30000 --heure 23 --armes 0.05 --threads $th "${DU[@]}" --verif 20 --etiquette threads >> $F
done
echo "assaut : fait"

# ------------------------------------------------------------ 4. Hiver
F=$D/resultats-hiver.csv
$B --mode hiver --entete > $F
H=(--colons 100000 --periode 200 --heure 0 --mono "${HIVER[@]}")
$B --mode hiver "${H[@]}" --temperature=2 --chrono $D/chrono-hiver-reference.csv --etiquette reference >> $F
$B --mode hiver "${H[@]}" --temperature=-5 --sans-chauffage --stocks 2000000,4000000,0,0,0 --livraisons 450000,1000000,0,0,0 --chrono $D/chrono-hiver-sans-rien.csv --etiquette sans-rien >> $F
$B --mode hiver "${H[@]}" --temperature=-5 --sans-chauffage --stocks 2000000,4000000,0,200000,200000 --livraisons 450000,1000000,0,0,0 --chrono $D/chrono-hiver-sans-medicaments.csv --etiquette sans-medicaments >> $F
$B --mode hiver "${H[@]}" --repr c --temperature=-5 --sans-chauffage --stocks 2000000,4000000,0,0,0 --livraisons 450000,1000000,0,0,0 --chrono $D/chrono-hiver-sans-rien-c.csv --etiquette sans-rien-c >> $F
echo "hiver : fait"

# ------------------------------------------------------------ 5. Sauvegarde
F=$D/resultats-sauvegarde.csv
$B --mode sauvegarde --entete > $F
$B --mode sauvegarde --colons 1000000 --periode 200 --heure 6 "${SAUVE[@]}" --etiquette sauvegarde >> $F
echo "sauvegarde : fait"

# ------------------------------------------------------------ 6. Micro-banc
cargo test --release banc_vivre -- --ignored --nocapture 2>/dev/null | grep "vivre :" > $D/banc-vivre.txt
echo "banc : fait"
