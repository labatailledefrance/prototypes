#!/usr/bin/env bash
# P-03 — crédibilité du mouvement, une tile réelle (la plus bâtie).
# Écrit resultats-observation.csv (une ligne toutes les 10 s de jeu) et les
# images de densité de passage dans observation/ (PNG), puis les planches de
# comparaison dans images/.
# Durée : environ 6 min. Les temps de calcul ne sont pas mesurés ici.
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --quiet
bin=target/release/p03-flow-fields
dossier=observation
rm -rf "$dossier"
mkdir -p "$dossier" images
out=resultats-observation.csv
"$bin" --observer x --entete > "$out"
RET=(--resolution 2 --regle majorite --periode-champ 20 --recalcul partiel --voisinage dense16 --reservation partout)
obs() {
  local sc=$1 duree=$2 fenetre=$3 etiquette=$4
  shift 4
  echo "[$(date +%H:%M:%S)] $etiquette" >&2
  "$bin" --scenario "$sc" --prechauffage 0 --observer "$dossier" --duree "$duree" --fenetre "$fenetre" \
    --etiquette "$etiquette" "${RET[@]}" "$@" >> "$out" 2>/dev/null
}

# Assaut : résolution, règle, marge, ligne droite.
for res in 1 2 4 5; do
  obs assaut 90 45 "assaut-${res}m" --resolution "$res"
done
obs assaut 90 45 assaut-5m-toute --resolution 5 --regle toute
obs assaut 90 45 assaut-2m-sans-marge --sans-marge
obs assaut 90 45 assaut-ligne --champ aucun
obs assaut 90 45 assaut-2m-contact --reservation contact
obs assaut 90 45 assaut-2m-aucune --reservation aucune
# Dispersé.
obs disperse 60 60 disperse-2m
obs disperse 60 60 disperse-5m --resolution 5
obs disperse 60 60 disperse-ligne --champ aucun
# Repli : période de recalcul du champ.
for per in 1 5 20 60; do
  obs repli 240 80 "repli-p${per}" --periode-champ "$per"
done
obs repli 240 80 repli-p20-sans-etalement --sans-etalement
obs repli 240 80 repli-ligne --champ aucun
# Barricade posée à 120 s pendant l'assaut.
obs assaut 240 120 barricade --barricade-a 120

python3 -I outils/ppm_vers_png.py "$dossier" > /dev/null
python3 -I outils/planches.py "$dossier" images
echo "Résultats : $out, $dossier/, images/" >&2
