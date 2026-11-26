# P-06 — Origine flottante et précision

Prototype jetable. Problèmes : [C-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-01--précision-numérique), [C-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-02--niveaux-de-vue), [C-03](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-03--globe), [A-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-01--nombre-de-zombies--simulation-par-tiles), [A-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-02--accélération-du-temps), [A-12](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-12--déterminisme-et-multijoueur), [A-13](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-13--sauvegardes), [F-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation), [G-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#g-01--frontières-avec-les-zones-non-simulées), [G-07](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#g-07--conditions-de-victoire-et-de-défaite). Fiche du prototype : [prototypes/README.md](../README.md#p-06--origine-flottante-et-précision). Premier prototype à utiliser le **rendu** de Bevy.

## Hypothèse testée

Le monde est à l'échelle 1:1 : la métropole et tous les outre-mer, et des navires sur toutes les mers. Un flottant 32 bits garde 24 bits de mantisse : à 500 km de l'origine, son pas est de 3 cm ; à 12 000 km (Paris–Nouvelle-Calédonie), de 1 m. Le GPU ne calcule qu'en 32 bits.

La piste de [C-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-01--précision-numérique) : coordonnées précises pour la simulation, **rendu relatif à la caméra** (origine flottante). Il faut :

1. choisir le **repère de la simulation** parmi trois :
   - **A** : un repère global unique en 64 bits (géocentrique, ECEF) ;
   - **B** : la grille projetée de chaque territoire en 64 bits (Lambert-93, UTM…), avec un passage d'un repère à l'autre en mer ;
   - **C** : des coordonnées « tile + position locale », la tile entière, la position en f32 ; un repère global pour la mer seulement ;
2. choisir la **stratégie de recalage** du rendu : à chaque frame, au-delà d'un seuil, ou la crate `big_space` ;
3. vérifier les **autres sources d'imprécision** : profondeur, ombres, temps de jeu dans les shaders, simulation à 20 Hz.

Le critère de la fiche : aucun tremblement visible, de Brest au point du territoire le plus éloigné. Il est rendu **mesurable** : écart en pixels et en millimètres entre la position calculée en 64 bits et la position réellement rendue.

Analogie : le GPU est un équipement qui n'accepte que des adresses sur 32 bits. Le monde a besoin de 64 bits. L'origine flottante est une traduction d'adresses (NAT) : chaque frame, ou de temps en temps, on ré-exprime les adresses par rapport à une passerelle proche de la caméra.

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, **NVIDIA RTX 3060 12 Go** (pilote 595.99, Vulkan), écran 3440 × 1440 en session Wayland, Linux 7.0. Rust 1.99, **Bevy 0.20.1** complet (même version que `bevy_ecs` 0.20.1 des prototypes précédents), `big_space` 0.14.0-rc.0. Compilation portable (sans `target-cpu=native`), `lto = "thin"`. Mesures du 2026-11-15.

Trois particularités de la machine, à connaître pour toute mesure de rendu :

- **Deux GPU** : une RTX 3050 sans écran est installée à côté de la 3060. Le script force la 3060 (`WGPU_ADAPTER_NAME="RTX 3060"`) ; le journal de Bevy le confirme (`AdapterInfo`).
- **La 3060 n'accepte que les surfaces Wayland** dans cette session (`vulkaninfo` : pas de `VK_KHR_xcb_surface` pour elle). Une fenêtre X11 (XWayland) fait paniquer Bevy (« Fallback system failed to choose present mode… Options: [] »). Les en-têtes de `libwayland` sont absents : `wayland-sys` est compilé en `dlopen`, qui charge la bibliothèque à l'exécution ([Cargo.toml](Cargo.toml)). Pas d'audio, de manettes ni d'udev (en-têtes absents, sans objet ici).
- **Sous Wayland, winit cale les frames sur le compositeur** (16,6 ms), même en `Immediate` ou `Mailbox` : impossible de désactiver la vsync d'une fenêtre. Le coût du rendu est donc mesuré **hors écran** (caméra rendue dans une image, sans winit, boucle à cadence libre).

## Méthode

### Géographie et repères ([src/geo.rs](src/geo.rs))

- Ellipsoïde GRS80 ; conversions géographique ↔ ECEF.
- **Projections** écrites à la main : Lambert conique conforme (Lambert-93, Lambert Nouvelle-Calédonie) et Mercator transverse par la série de Krüger à l'ordre 6 (UTM). Elles sont **validées contre PROJ** (`cs2cs`, installé sur la machine) : voir [resultats-proj.csv](resultats-proj.csv).
- **Zones de projection** (codes EPSG vérifiés dans la base de PROJ) :

| Territoire | Projection | EPSG |
|---|---|---|
| Métropole, Corse comprise | RGF93 / Lambert-93 | 2154 |
| Guadeloupe, Martinique, Saint-Martin, Saint-Barthélemy | RGAF09 / UTM 20N | 5490 |
| Guyane | RGFG95 / UTM 22N | 2972 |
| Réunion | RGR92 / UTM 40S | 2975 |
| Mayotte | RGM04 / UTM 38S | 4471 |
| Saint-Pierre-et-Miquelon | RGSPM06 / UTM 21N | 4467 |
| Polynésie (5 fuseaux) | RGPF / UTM 5S, 6S (Tahiti, Rapa), 7S (Marquises), 8S (Gambier) | 3296 à 3299 |
| Nouvelle-Calédonie | RGNC91-93 / Lambert Nouvelle-Calédonie | 3163 |
| Wallis-et-Futuna | RGWF96 / UTM 1S | 8903 |
| Kerguelen, Crozet, Amsterdam (TAAF, mesurées puis exclues du jeu) | RGTAAF07 / UTM 42S, 39S, 43S | 7079, 7076, 7080 |

  Les codes existent dans PROJ ; que ce soient **les projections officielles d'usage** pour chaque territoire (en particulier le fuseau de chaque archipel polynésien) reste **à vérifier**. Terre Adélie (stéréographique polaire) et Clipperton ne sont pas traités ; les TAAF et Clipperton sont depuis exclus du jeu.
- **27 lieux de référence** ([geo.rs](src/geo.rs), coordonnées approchées) : les extrêmes de la métropole, les chefs-lieux et les îles extrêmes de chaque territoire. Le **point le plus éloigné de Paris** est calculé : c'est l'île Walpole (Nouvelle-Calédonie).

**Repère d'une tile.** Une tile est `(zone, ix, iy)` sur la grille de 1 km de sa projection ; les coordonnées locales vont de 0 à 1 000 m. Pour placer la tile sur la Terre, deux approximations sont comparées au calcul exact (projection inverse + ECEF, en chaque point) :

- **affine** : plan tangent au centre de la tile (jacobien de la projection inverse et verticale de l'ellipsoïde) ;
- **quadratique** : la même, plus les termes du second ordre (développement de Taylor au centre, dérivées secondes par différences finies), quelques multiplications de plus par point.

### Simulation selon le repère ([src/simu.rs](src/simu.rs))

Des marcheurs à 1,4 m/s, à 20 Hz, dans les 10 × 10 tiles autour d'un lieu. À chaque tick : un virage de 1° selon un tirage haché (identifiant, tick) et la case de 2 m d'un champ par tile (comme un flow field de [P-03](../p03-flow-fields/README.md)), puis un pas. Même marche dans les trois repères :

- **A** : position ECEF en f64 (24 octets) ramenée sur la sphère, cap tangent ; la tile et la case du champ exigent une **projection à chaque tick** ;
- **B** : position dans la grille du territoire en f64 (16 octets) ; tile et case par troncature ;
- **C** : tile entière + position locale en f32 (8 octets), **centrée sur la tile** (±500 m) ; changement de tile par soustraction de 1 000 m.

Systèmes Bevy (`par_iter_mut` avec taille de lot minimale, F-02). Somme de contrôle des positions (bits exacts) : de 1 à 16 threads, lots de 64 à 100 000, même départ local en différents points du monde, et avec le rendu relatif à la caméra actif dans le même `App`, caméra à Brest, Tahiti ou Walpole.

### Rendu relatif à la caméra ([src/monde.rs](src/monde.rs))

Chaque modèle a une **ancre** : une tile et une position locale f32 (soldats, bâtiments, sol), ou une position ECEF f64 (navire en mer, morceaux du globe, caméra). La caméra a une pose ECEF f64. Une **stratégie** transforme ces poses en `Transform` f32 ; Bevy les propage en `GlobalTransform`.

| Stratégie | Ce qu'elle fait |
|---|---|
| `naif-paris` | **témoin** : monde f32 à origine fixe à Paris (axes locaux de Paris) |
| `naif-ecef` | **témoin** : monde f32 à origine fixe au centre de la Terre |
| `chaque-frame` | origine posée sur la caméra à chaque frame, tous les modèles recalculés |
| `seuil:S` | origine reposée sur la caméra quand elle s'en éloigne de plus de S m ; entre deux, seuls les modèles qui bougent sont recalculés |
| `decalage:S` | origine maintenue à S m de la caméra : le pire cas d'un seuil S |
| `tiles`, `tiles-seuil:S` | une entité parente par tile, recalée seule ; les modèles sont ses enfants, en position locale (hiérarchie de Bevy) |
| `big-space:E` | crate `big_space` : cellules entières (i64) de E m, origine flottante sur la caméra ; les modèles immobiles sont marqués `Stationary` |

Les axes du rendu sont ceux du repère local de la caméra (x à l'est, y en haut), sauf `naif-ecef` et `big_space` (axes géocentriques fixes).

**Relecture de la projection** (système `mesurer`). Après la propagation de Bevy, pour chaque point sondé, le prototype rejoue en f32 exactement ce que font Bevy et le GPU : matrice du modèle tirée du `GlobalTransform` réellement calculé, `clip_from_world` = projection × inverse de la matrice de la caméra (comme `bevy_render`), division perspective. Il compare à la projection f64 de la position de référence. Il en tire :

- l'**écart** en pixels (écran de 1 920 × 1 080, champ vertical de 30°) et en millimètres dans le repère de la caméra ;
- le **tremblement** : la variation de l'écart d'une frame à la suivante, en pixels. C'est lui que l'œil voit bouger.

Points sondés : le soldat (7 points), le sol de sa tile et de la voisine, un bâtiment, la coque et le mât du navire, un sommet du globe.

**Validation par le rendu réel** (mode `capture`). Une sphère magenta non éclairée est posée au-dessus de la tête du soldat ; tout le reste a d'autres couleurs. La simulation est figée, le rendu rattrape l'état, une capture d'écran est prise, et le **centroïde des pixels magenta** est comparé à la projection f64 du centre de la sphère et à la relecture. Le centroïde d'une sphère rastérisée a lui-même un biais de quelques centièmes de pixel : c'est le plancher de cette mesure.

### Scénarios ([run.sh](run.sh))

| Scénario | Mesure |
|---|---|
| 1. Soldat qui marche, caméra « isométrique » à 25 m (35° de plongée, dans l'axe de la rue), **même instant de jeu** (jour 20) à Brest, Tahiti et l'île Walpole | 60 s de jeu (3 600 frames) ; le soldat change de tile au bout de 14 s |
| 2. Même scène, témoins naïfs | idem |
| 3. Navire Brest → Iroise → Antilles → Colón → Balboa (canal non simulé) → Papeete → Nouméa, à 8 m/s, accéléré ×20 000, caméra à 400 m | 32 000 frames ; ancre de tile à moins de 30 km d'un port (zone du port), ancre ECEF en mer ; saut de position aux changements d'ancre ; passage de l'antiméridien |
| 4. Zoom continu du soldat (25 m) au globe (30 000 km) et retour, à Tahiti et Walpole | 7 200 frames ; résolution de la profondeur à chaque distance ; sommet du globe découpé et du globe naïf |
| 5. Partie longue : jour 42, ×100 | positions ; horloge des shaders analysée à part ([src/analyse.rs](src/analyse.rs)), de 1 h à 10 ans de jeu |
| Profondeur | deux plaques parallèles (verte dessous, rouge dessus) vues à 35°, de 30 m à 20 000 km, écart de 0,1 mm à 10 km, rendu réel : part de vert visible au centre de l'image |
| Ombres | une scène éclairée avec ombres en cascade, caméra immobile ; image rendue pour une origine à 0, 10 m, 100 m… 1 000 km de la caméra, et pour les témoins ; pixels qui changent par rapport à l'origine sur la caméra (seuil de 8/255), avec et sans ombres |
| Coût | 10 000, 100  000 et 1 million de cubes répartis sur 10 × 10 tiles, 1 % de mobiles qui bougent à chaque frame, caméra à 1 km/s (un recalage par seconde au seuil de 1 km) ; chronos du recalage et de la propagation de Bevy (`TransformSystems::Propagate`) ; trois répétitions |
| Cadence | mêmes cubes, rendus hors écran : temps de frame complet (extraction, préparation, soumission au GPU) |

Les modèles sont synthétiques : capsules, blocs de 8 à 16 m de côté et de 6 à 30 m de haut le long d'une rue, sol de 9 × 9 tiles, coque de 120 m, globe en 1 536 morceaux (cube projeté sur l'ellipsoïde, 16 × 16 morceaux par face, chacun avec son origine locale).

### Seuil d'invisibilité retenu

Un écart est jugé **invisible sous 0,5 pixel**, et un tremblement sous **0,25 pixel d'une frame à l'autre**. Justification :

- le GPU place les sommets sur une grille de 1/256 de pixel, mais l'œil ne voit un objet bouger que quand ses bords changent de pixel ;
- un écart fixe de moins d'un demi-pixel ne change le pixel d'un bord que pour une fraction des bords, une seule fois, sans mouvement ;
- un tremblement de 0,25 px fait au pire scintiller un pixel de bord sur quatre, à 60 images par seconde ; la mesure par captures a d'ailleurs un plancher de 0,01 à 0,1 px.

À la vue rapprochée (25 m, 30°), un pixel couvre 1,24 cm au sol : 0,5 px ≈ 6 mm.

### Paramètres de prototype

Ce sont des **hypothèses de mesure**, pas des statistiques de jeu : écran de 1 920 × 1 080, champ vertical de 30°, plan proche à 0,1 m ; caméra à 25 m du soldat (400 m du navire) ; soldat à 1,4 m/s ; navire à 8 m/s ; 1 % de mobiles ; caméra à 1 km/s dans les mesures de coût ; soleil à 40° d'élévation, cascades d'ombres jusqu'à 400 m.

Reproduire : `./run.sh` (environ 25 min, ouvre des fenêtres pendant la partie « rendu réel ») ; `ESSAI=1 ./run.sh` valide toutes les commandes en 3 min. Vérification à l'œil : `./voir.sh` (liste des scènes), `./voir.sh tout`. Tests : `cargo test --release` (7 tests) ; micro-bancs : `cargo test --release banc -- --ignored --nocapture`.

## Budgets de référence

Les mêmes que pour P-00 à P-05 : 25 ms par tick de simulation à ×1, 2,5 ms à ×10, 0,25 ms à ×100. Pour le rendu, la frame de 16,6 ms à 60 images par seconde, dont l'autre moitié (8,3 ms) n'est pas à la simulation : le recalage et la propagation des `Transform` doivent y rester une petite part.

## Mesures

### 1. Géodésie ([resultats-geo.csv](resultats-geo.csv), [resultats-proj.csv](resultats-proj.csv))

**Projections** : écart avec PROJ inférieur à 0,1 mm sur les 27 lieux. En Polynésie, `cs2cs` applique en plus la transformation WGS84 → RGPF (0,6 à 0,8 m, précision annoncée 0,5 m) ; la projection seule est exacte. Aller-retour géographique → grille → géographique : moins de 0,003 mm.

**Point le plus éloigné de Paris : l'île Walpole** (Nouvelle-Calédonie, inhabitée), à 16 866 km (corde de 12 355 km). Suivent l'île des Pins (16 817 km), Nouméa (16 744 km), Futuna (16 172 km), Wallis (16 058 km).

Pas d'un flottant selon le repère (le pas f32 est la plus petite différence représentable ; l'arrondi en vaut la moitié) :

| Lieu | Distance de Paris | Pas f32, origine à Paris | Pas f32, ECEF | Pas f32, grille du territoire | Pas f64, grille |
|---|---|---|---|---|---|
| Brest | 505 km | 3 cm | 50 cm | 50 cm | 0,9 nm |
| Bonifacio | 986 km | 6 cm | 50 cm | 50 cm | 0,9 nm |
| Fort-de-France | 6 856 km | 50 cm | 50 cm | 12 cm | 0,2 nm |
| Papeete | 15 712 km | 1 m | 50 cm | 50 cm | 0,9 nm |
| Île Walpole | 16 866 km | 1 m | 50 cm | 6 cm | 0,1 nm |
| Mata-Utu | 16 058 km | 1 m | 50 cm | 1 m | 1,9 nm |

**Facteur d'échelle k** (longueur de la grille / longueur au sol) : de 0,99959 (Brest) à 1,0029 (Bonifacio) en Lambert-93 ; au plus 1,0005 outre-mer avec le bon fuseau. Rapa (144,3° O) relève de l'UTM 6S et les Gambier de l'UTM 8S : placées en 7S, Mangareva avait un k de 1,0043.

**Repère d'une tile** (écart au calcul exact sur une grille de 21 × 21 points de la tile, et entre les bords de deux tiles voisines) :

| | Écart vertical | Écart horizontal | Jointure entre tiles | Coût par modèle (un thread) |
|---|---|---|---|---|
| Affine (plan tangent) | **39 mm** aux coins, partout | 0,03 à 3,4 mm | 0,05 à 6,7 mm | 2,2 ns |
| **Quadratique** (second ordre) | ≤ 0,005 mm | | **≤ 0,007 mm** | 9,8 ns |

Les 39 mm sont la flèche de la Terre sur 707 m (demi-diagonale d'une tile) : d²/2R.

### 2. Précision du rendu, relecture de la projection ([resultats-precision.csv](resultats-precision.csv))

**Scénarios 1 et 2 : soldat qui marche**, 60 s, 3 600 frames, même instant de jeu. Écart maximal en pixels sur les sondes du soldat, du sol et du bâtiment ; entre parenthèses, en millimètres ; puis tremblement maximal.

| Stratégie | Brest | Papeete | Île Walpole | Tremblement max |
|---|---|---|---|---|
| témoin, origine à Paris | **8,4 px** (120 mm) | **189 px** (2,7 m) | **298 px** (3,4 m) | 8 / 202 / 346 px |
| témoin, origine ECEF | **157 px** (1,8 m) | **141 px** (1,8 m) | **188 px** (1,8 m) | 135 / 175 / 133 px |
| recalage à chaque frame | 0,004 px (0,05 mm) | 0,004 px | 0,003 px | 0,004 px |
| seuil de 1 km | 0,002 px (0,02 mm) | 0,003 px | 0,002 px | 0,002 px |
| hiérarchie par tiles | 0,011 px (0,14 mm) | 0,006 px | 0,009 px | 0,013 px |
| `big_space`, cellules de 1 km | 0,010 px (0,15 mm) | 0,009 px | 0,012 px | 0,014 px |

Les témoins tremblent comme prévu : l'erreur suit le pas du flottant (3 cm à Brest, 1 m outre-mer), amplifiée par l'inversion en f32 de la matrice de la caméra que fait `bevy_render`.

**Distance caméra–origine imposée** (pire cas d'un seuil), maximum sur les trois lieux :

| Distance | 100 m | 1 km | 10 km | 100 km | 1 000 km |
|---|---|---|---|---|---|
| Écart max | 0,005 px | 0,016 px | 0,13 px (1,5 mm) | **1,3 px** (17 mm) | **9,2 px** (117 mm) |
| Tremblement max | 0,005 px | 0,016 px | 0,15 px | 0,98 px | 7,5 px |

**Scénario 3 : navire** Brest → Panama → Papeete → Nouméa (17 900 km), ×20 000, caméra à 400 m :

| Stratégie | Écart max | Tremblement max |
|---|---|---|
| témoin, origine à Paris | 13 px (3,8 m) | 23 px |
| témoin, origine ECEF | 10 px (1,8 m) | 15 px |
| recalage à chaque frame | 0,0004 px (0,1 mm) | 0,0005 px |
| seuil de 1 km (8 044 recalages : le navire fait 2,7 km par frame) | 0,0004 px | 0,0005 px |
| `big_space` | 0,001 px (0,2 mm) | 0,001 px |

**Changements de représentation** (tile du port ↔ ECEF en mer, à 30 km de Brest, de Papeete et de Nouméa) : saut de **0,0 mm** avec le repère quadratique. Avec le repère affine (première version), la sortie de Brest sautait de 19,4 mm (0,10 px à 400 m). Le passage de l'**antiméridien** (Papeete → Nouméa) ne produit rien de particulier : l'ECEF n'a pas de longitude.

**Scénario 4 : zoom** de 25 m à 30 000 km et retour ([chrono-zoom-*.csv](chrono-zoom-Papeete-chaque-frame.csv)). Écart maximal par tranche de distance, à Papeete :

| Distance caméra | 10–100 m | 100 m–1 km | 1–10 km | 10–100 km | ≥ 100 km |
|---|---|---|---|---|---|
| Soldat et sol, recalage à chaque frame | 0,0015 px | 0,0008 px | 0,0004 px | 0,0004 px | 0,0004 px |
| Soldat, témoin origine à Paris | **139 px** | 36 px | 3,1 px | 0,36 px | < 0,04 px |
| Globe découpé (morceaux de 625 km), chaque frame | **3,3 px** | 1,0 px | 0,11 px | 0,01 px | < 0,002 px |
| Globe découpé, seuil de 1 km | 0,72 px | 0,20 px | 0,08 px | 0,008 px | < 0,002 px |
| Globe non découpé (sommets relatifs au centre de la Terre) | **74 px** | 19 px | 2,2 px | 0,19 px | < 0,03 px |

Même résultat à l'île Walpole. Avec le recalage à chaque frame, le morceau de globe **tremble** (1,2 px au p99 de près) ; au seuil, son erreur est fixe (tremblement de 0,002 px).

**Scénario 5 : partie longue** (jour 42, ×100, Walpole) : écart max de 0,003 px (chaque frame) et 0,014 px (seuil, 9 recalages). La durée de la partie n'a aucun effet sur les positions : l'horloge est entière.

### 3. Validation par le rendu réel ([resultats-captures.csv](resultats-captures.csv))

Centroïde du marqueur magenta dans 60 captures (moins pour les témoins quand le marqueur est déformé ou masqué), comparé à la projection f64.

| Stratégie | Brest (méd. / max) | Papeete | Île Walpole |
|---|---|---|---|
| témoin, origine à Paris | **1,7 / 4,5 px** | **50 / 143 px** | **105 / 208 px** |
| témoin, origine ECEF | 42 / 63 px | 31 / 132 px | 36 / 88 px |
| recalage à chaque frame | 0,07 / 0,09 px | 0,04 / 0,10 px | 0,03 / 0,06 px |
| seuil de 1 km | 0,07 / 0,09 px | 0,05 / 0,10 px | 0,03 / 0,07 px |
| `big_space` | 0,07 / 0,09 px | 0,04 / 0,11 px | 0,03 / 0,07 px |

Avec recalage, l'écart GPU–relecture (0,03 à 0,1 px) est le biais du centroïde d'une sphère rastérisée : c'est le **plancher** de cette mesure, et la relecture est confirmée en dessous. Pour les témoins, le GPU retrouve l'ordre de grandeur de la relecture ; l'écart entre les deux (0,6 px à Brest, 20 à 45 px outre-mer) vient de la déformation du marqueur lui-même (sommets arrondis au mètre : la sphère devient un carré, voir [images/ombres-1-naif-paris.png](images/ombres-1-naif-paris.png)) et des FMA du GPU.

### 4. Profondeur ([resultats-profondeur.csv](resultats-profondeur.csv))

Plus petit écart entre deux plaques parallèles sans z-fighting (moins de 0,1 % de pixels de la plaque du dessous), rendu réel, profondeur inversée de Bevy (f32, plan lointain infini, plan proche à 0,1 m) :

| Distance | 30 m | 300 m | 3 km | 30 km | 300 km | 3 000 km | 20 000 km |
|---|---|---|---|---|---|---|---|
| Résolution de la profondeur (calculée) | 0,002 mm | 0,03 mm | 0,3 mm | 2 mm | 2,6 cm | 32 cm | 1,8 m |
| Écart net, recalage | ≤ 0,1 mm | ≤ 0,1 mm | 1 mm | 1 cm | 10 cm | 1 m | 10 m |
| Écart net, témoin origine à Paris (Papeete) | — (échecs à 1 et 10 cm) | 10 cm | 1 m | 10 cm | 1 m | 1 m | 10 m |

Avec recalage, l'écart net vaut environ **3 × 10⁻⁷ fois la distance** : la profondeur inversée en f32 a une précision relative constante, et les sommets relatifs à la caméra aussi. Le témoin arrondit les sommets au mètre et z-fighte jusqu'à 1 m d'écart dès quelques kilomètres.

### 5. Ombres ([resultats-ombres.csv](resultats-ombres.csv), [images/](images))

Part des pixels de l'image qui changent (de plus de 8/255) quand l'origine est déplacée de la caméra, caméra et scène immobiles :

| Origine à | 10 m | 100 m | 1 km | 10 km | 100 km | 1 000 km | Paris (témoin) | Centre de la Terre (témoin) |
|---|---|---|---|---|---|---|---|---|
| Sans ombres, Papeete | 0 | 0 | 0,0002 % | 0,002 % | 0,02 % | 0,08 % | 4,6 % | 2,6 % |
| Ombres en cascade, Papeete | 0,21 % | 0,20 % | 0,01 % | 0,03 % | 0,35 % | 0,99 % | **19 %** | **14 %** |
| Ombres en cascade, Brest | 0,47 % | 0,32 % | 0,29 % | 0,19 % | 0,54 % | 2,95 % | 2,1 % | **21 %** |

Sans ombres, l'image ne change pas jusqu'à 100 m et à peine jusqu'à 100 km. **Avec ombres, elle change dès 10 m, sans croître avec la distance** : ce n'est pas une perte de précision, c'est la grille de texels des cascades qui se cale sur l'origine du monde. Les bords d'ombre se redessinent autrement (0,2 à 0,5 % des pixels, sur les bords). Le témoin couvre les ombres d'acné.

### 6. Horloge des shaders ([resultats-temps.csv](resultats-temps.csv))

Défilement d'une texture à 1 m/s et houle de 7 s (0,5 m), calculés en f32 comme dans un shader, comparés au calcul exact :

| Durée de partie | 1 h | 1 jour | 7 jours | 28 jours | 1 an | 10 ans |
|---|---|---|---|---|---|---|
| Pas du temps f32 | 0,24 ms | 7,8 ms | 62 ms | 0,25 s | 2 s | 32 s |
| Écart du défilement, f32 absolu, ×1 | 0,3 mm | **9,5 mm (0,8 px)** | 53 mm | 210 mm | 2,5 m | — |
| Frames figées, f32 absolu, ×1 | 0 | 0 | **73 %** | 93 % | 99 % | 99,8 % |
| Phase calculée sur les ticks | < 0,001 mm | < 0,001 mm | < 0,001 mm | < 0,001 mm | < 0,001 mm | < 0,001 mm |

- **f32 absolu** : visible (plus de 0,5 px) à partir de **36,4 h de jeu**, soit 22 min de temps réel à ×100. Au-delà d'une semaine, la plupart des frames ne voient plus le temps avancer à ×1 : les animations se figent par saccades.
- **f32 modulo 3 600 s** (le `globals.time` de Bevy, qui boucle toutes les heures) : lisse, mais une animation dont la période ne divise pas 3 600 s saute au bouclage (houle de 7 s : saut de 0,49 m), soit toutes les 36 s de temps réel à ×100.
- **Phase calculée sur le CPU** à partir des ticks entiers (`tick mod période`), envoyée en f32 dans [0, 1) : aucune erreur, quelle que soit la durée.

Le compteur de ticks lui-même : en u64 dans ce prototype et dans P-04 ; mais la fiche d'un colon de [P-05](../p05-civils/src/colon.rs) date sa mise à jour en **u32**, qui déborde après 2³² ticks = **6,8 ans de jeu** à 20 Hz (25 jours de temps réel à ×100).

### 7. Coût du recalage ([resultats-cout.csv](resultats-cout.csv))

Recalage + propagation des `Transform` par Bevy, sans rendu, 16 threads, médiane de 600 frames (trois répétitions : plage des médianes) ; « pic » = frames où l'origine change (une par seconde : caméra à 1 km/s, seuil de 1 km).

| Modèles | Chaque frame | Seuil : courant / pic | Hiérarchie par tiles, chaque frame | Hiérarchie, seuil | `big_space` : courant / pic |
|---|---|---|---|---|---|
| 10 000 | 0,32 ms | **0,07** / 0,32–0,38 ms | 0,21–0,23 ms | 0,16 ms | 0,12 / 0,19–0,20 ms |
| 100 000 | 2,2–2,3 ms | **0,24–0,25** / 2,3 ms | 0,92–1,37 ms | 1,2–1,35 ms | 0,27–0,28 / 0,84 ms |
| 1 million | **26–28 ms** | **1,6** / 26–27 ms (max 30) | 30–35 ms | 25–32 ms | 1,7–2,0 / **10,7–11,3 ms** (max 13) |

À 1 million, sans aucun mobile : la hiérarchie au seuil tombe à 1,6 ms en courant (pic 34 ms). **Un seul enfant qui bouge fait repropager tous les enfants de son parent** : avec 1 % de mobiles répartis sur 100 tiles, chaque parent est touché à chaque frame.

Micro-banc ([banc.txt](banc.txt), un thread, hors ECS) : calcul d'une position relative à l'origine, 2,2 ns (affine) et 9,8 ns (quadratique) par modèle. Le coût mesuré dans l'ECS (environ 10 ns par modèle sur 16 threads) est donc surtout celui des écritures et de la détection de changement, puis de la propagation.

### 8. Cadence de rendu ([resultats-cadence.csv](resultats-cadence.csv))

Temps de frame complet, rendu hors écran (cubes d'un même maillage, 1 % de mobiles, caméra à 1 km/s), 600 frames :

| Modèles | Chaque frame (méd. / p99) | Seuil de 1 km | `big_space` |
|---|---|---|---|
| 10 000 | 2,6 / 3,8 ms | 1,6 / 2,8 ms | 1,7 / 3,5 ms |
| 100 000 | **12,0 / 16,4 ms** | **3,4 / 6,5 ms** (max 13,8) | 3,8 / 7,2 ms (max 10,1) |
| 1 million | 97 / 152 ms | 22 / **111 ms** | 23 / **104 ms** |

Recaler à chaque frame coûte deux fois : dans la propagation, puis dans le rendu, qui réextrait et renvoie au GPU toutes les transformations modifiées. À 100 000 modèles, le recalage permanent prend 8,6 ms de frame de plus que le seuil. Au seuil, la frame du recalage coûte autant qu'une frame « chaque frame » : c'est un à-coup, rare.

### 9. Simulation selon le repère ([resultats-simu.csv](resultats-simu.csv), [resultats-derive.csv](resultats-derive.csv), [resultats-sauvegarde.csv](resultats-sauvegarde.csv))

1 million de marcheurs, 200 ticks, médiane par tick :

| Repère | Octets par position | 1 thread | 16 threads | Banc (ns par marcheur) |
|---|---|---|---|---|
| **C** tile + local f32 | **8** | 14,1 ms | **2,3 ms** | 14 |
| **B** grille f64 | 16 | 16,3 ms | 2,6 ms | 16 (26 avec `floor`) |
| **A** ECEF f64 | 24 | 321 ms | **42 ms** | 318 |

**Déterminisme** (somme de contrôle des bits des positions) :

- chaque repère donne la même somme de 1 à 16 threads et pour des lots de 64 à 100 000 ;
- **C donne la même somme à Brest, Papeete, Walpole et Nouméa** (même départ local) : son calcul ne dépend pas de l'endroit du monde ;
- B donne la même somme à Brest et Papeete (mêmes exposants des coordonnées), **pas** à Walpole ni à Nouméa ; A diffère partout ;
- **avec le rendu relatif à la caméra dans le même `App`**, caméra à Brest, Papeete ou Walpole : la somme de C ne change pas.

**Passage de tile** en C : exact dans les deux sens avec des coordonnées centrées (±500 m, lemme de Sterbenz : la soustraction de deux flottants proches est exacte), vérifié par test sur 10 000 valeurs de chaque côté. Depuis le coin (0 à 1 000 m), le passage vers l'ouest arrondit (−0,03 + 1 000 n'est pas représentable).

**Dérive** sur 100 km en ligne droite (pas f32 accumulés) : C s'écarte de 7,7 m à 1,4 m/s (biais de vitesse de 0,008 %), 46 m à 0,3 m/s (0,05 %), 82 m à 0,05 m/s (0,08 %) ; B, 0. Le biais vient de l'arrondi du pas au pas de la position (au plus 3 × 10⁻⁵ m à 500 m).

**Sauvegarde** de 12 300 positions d'une tile, après 600 ticks de marche, triées, écarts en varint, zstd :

| Représentation | Brut | zstd |
|---|---|---|
| C, u16 au 1/64 m (A-13) | 4 o | **3,2 o** |
| B, i32 au 1/64 m | 8 o | 3,2 o |
| A, 3 × i32 au 1/64 m | 12 o | 4,9 o |
| C, f32 exacts | 8 o | 6,2 o |
| B, f64 exacts | 16 o | 10,4 o |
| A, f64 exacts | 24 o | 16,8 o |

Quantifiées, B et C coûtent la même chose : seuls comptent les écarts entre positions voisines. A paie la troisième coordonnée (la hauteur est mélangée aux trois axes).

## Analyse

**1. Le témoin tremble, et la mesure le voit.** Origine à Paris : 8 px à Brest, 190 px à Papeete, 300 px à Walpole ; origine au centre de la Terre : 140 à 190 px partout. Les captures du GPU donnent les mêmes ordres de grandeur (1,7 / 50 / 105 px en médiane). Avec n'importe quel recalage, l'écart tombe sous 0,02 px par la relecture et sous le plancher de 0,1 px des captures. La mesure n'est donc pas fausse, et **le rendu relatif à la caméra résout C-01 partout, y compris au point le plus éloigné**.

**2. Le repère de la simulation : C.** Analogie : A est un adressage global en IPv6 pour chaque paquet interne d'un réseau local ; C est un réseau privé par site (la tile), avec une traduction à la frontière.

- **A (ECEF)** n'a pas de sol : ni « haut » constant, ni grille. Les tiles, le k-d tree et les flow fields de 2 m sont dans un plan projeté ; A doit **projeter à chaque tick** pour savoir dans quelle tile et quelle case il se trouve. C'est 20 fois plus cher (318 ns contre 14 par marcheur), ce qui tient mal dans le budget même avec 16 threads (42 ms par tick pour 1 million). Ses fonctions trigonométriques viennent en outre de la bibliothèque du système : déterminisme entre plateformes à reconstruire (A-12). Sa sauvegarde est la plus lourde. Il ne sert qu'en mer.
- **B (grille f64)** marche : précision de l'ordre du nanomètre, coût proche de C. Mais il double la mémoire de chaque position (16 octets), alors que P-00 a montré que la bande passante mémoire est la limite. Son résultat dépend de l'endroit (somme différente à Walpole) : un même combat rejoué ailleurs ne donne pas les mêmes bits, ce qui gêne les tests. Un `floor` y coûte un appel de bibliothèque (26 ns contre 16, le piège de P-05) si l'on oublie que les coordonnées sont positives.
- **C (tile + local f32)** est le moins cher, le plus compact, et **invariant par translation** : la même scène donne les mêmes bits à Brest et à Walpole. C'est déjà ce que font P-01 à P-05 (positions `[f32; 2]` locales à la tile) et ce que sauvegarde A-13 (1/64 m dans la tile). Son seul défaut mesuré, un biais de vitesse de 0,01 à 0,08 %, est invisible.
- **Les flow fields de 2 m, le k-d tree et la sauvegarde** restent tels quels en C : ils sont déjà locaux à la tile. En B, ils recalculent leur origine à chaque accès ; en A, ils exigent une projection.

**3. Le passage d'un territoire à l'autre se fait par l'ECEF, et le repère de tile doit être quadratique.** Un navire qui quitte une zone convertit sa position de tile en ECEF (projection inverse exacte), navigue en ECEF f64, et se reconvertit à l'arrivée dans la zone du port. Le plan tangent (affine) est faux de 39 mm aux coins de chaque tile : le navire sautait de 19 mm à la sortie de Brest, et deux tiles voisines ne se raccordaient qu'à 7 mm près. Le second ordre ramène tout sous 0,01 mm, pour 7,6 ns de plus par modèle recalé. L'antiméridien et les cinq fuseaux de la Polynésie ne posent aucun problème : en mer, tout est en ECEF ; à terre, chaque fuseau est une zone.

**4. Le recalage : au seuil, pas à chaque frame.** La précision ne départage pas les stratégies (toutes sous 0,02 px). Le coût et les effets secondaires, si.

- **À chaque frame**, tout modèle est réécrit, repropagé et renvoyé au GPU à chaque frame : 2,3 ms de CPU et 8,6 ms de frame de plus à 100 000 modèles, 26 ms et 75 ms à 1 million. Il fait aussi **trembler les grands maillages dont l'origine est loin** (morceaux de globe : l'arrondi de leur origine change à chaque frame). C'est comme renuméroter toutes les adresses du réseau à chaque paquet.
- **Au seuil**, seuls les modèles qui bougent sont réécrits (0,25 ms à 100 000) ; l'erreur est figée entre deux recalages, donc ne tremble pas. Le prix : une frame de recalage aussi chère qu'une frame « chaque frame » (2,3 ms à 100 000, 26 ms à 1 million, et 104 à 111 ms de frame à 1 million). Le seuil se choisit sur la courbe des décalages : jusqu'à 10 km, l'écart reste sous 0,15 px. Un seuil de quelques kilomètres rend l'à-coup rare (une fois toutes les quelques minutes pour une caméra qui suit une colonne) et garde une marge de plus de 10.
- **La hiérarchie par tiles** est une fausse bonne idée dans Bevy : le moindre enfant mobile fait repropager tous ses frères, et la propagation hiérarchique coûte deux fois la propagation à plat de tous les modèles (30 ms contre 16 ms à 1 million).
- **`big_space`** est précis et le plus doux au recalage (11 ms au lieu de 27 à 1 million en CPU, grâce à une propagation parallèle en f64), mais le rendu paie le même à-coup (104 ms contre 111). Il n'est **pas mûr** pour ce projet : version candidate (0.14.0-rc.0) pour Bevy 0.20, alors que la 0.13 visait la 0.19 ; un seul mainteneur ; il **impose de désactiver le `TransformPlugin` de Bevy** et remplace toute la propagation ; ses axes sont géocentriques fixes, si bien que « haut » n'est plus +Y à Tahiti (lumières, caméras, physique, outils de l'écosystème en dépendent) ; et il faut marquer chaque modèle immobile `Stationary`, sinon il repropage tout à chaque frame. Le recalage au seuil fait le même travail en une centaine de lignes du jeu.

**5. Le vrai coût est le nombre d'entités rendues, pas la méthode.** À 1 million de modèles, aucune stratégie ne tient la frame (22 ms de médiane au seuil, avant tout recalage). Le recalage ne fait qu'ajouter un à-coup proportionnel au nombre d'entités qui portent un `Transform` de rendu. Les foules doivent donc passer par l'instanciation (P-11), avec un tampon de positions relatives à l'origine rempli directement depuis la simulation, et non par une entité rendue par zombie.

**6. Le globe se découpe, et le sol des tiles prend le relais de près.** Un globe dont les sommets sont relatifs au centre de la Terre est faux de 74 px de près et de 2 px à 1 km. Découpé en morceaux de 625 km à origine locale, il est sous 0,5 px au-delà de 1 à 10 km de la caméra ; plus près, c'est le sol des tiles qui doit être affiché (C-02, C-03).

**7. La profondeur ne pose aucun problème.** Avec la profondeur inversée de Bevy et des sommets relatifs à la caméra, deux surfaces se départagent dès 3 × 10⁻⁷ fois la distance : 0,1 mm à 30 m, 1 cm à 30 km, 10 m vu de l'espace. Rien à faire pour le z-fighting au-delà de l'origine flottante ; les surfaces superposées (marquages au sol, eau sur la rive) gardent un écart d'au moins 10⁻⁶ fois la distance de vue maximale où elles sont affichées.

**8. Les ombres bougent à chaque recalage, quelle que soit la distance.** Sans ombres, un recalage de 1 km ne change que 0,0002 % des pixels ; avec les cascades de Bevy, 0,2 à 0,5 % (des bords d'ombre), même pour 10 m. Au seuil, cela fait un léger saut des bords d'ombre à chaque recalage ; à chaque frame, un scintillement permanent quand la caméra bouge. C'est un argument de plus pour le seuil, et une question pour le rendu (cascades calées sur leur grille de texels).

**9. Le temps des shaders ne doit jamais être un f32 absolu.** Visible après 36 h de jeu, figé par saccades après une semaine : avec une partie sans fin (G-07) et ×100, cela arrive dans la première heure de jeu réelle. Le `globals.time` de Bevy (modulo 3 600 s) est lisse mais saute toutes les heures de jeu pour toute période qui ne divise pas 3 600 s. Les phases calculées sur le CPU depuis les ticks entiers sont exactes indéfiniment.

**10. Pièges trouvés, à retenir pour le jeu.**

- **Compilation asynchrone des shaders** : Bevy ne dessine rien tant que les pipelines ne sont pas compilés (quelques secondes). Toute capture ou mesure de rendu doit attendre en temps réel, pas un nombre de frames.
- **Sous Wayland, une fenêtre est cadencée par le compositeur** : les temps de frame se mesurent hors écran.
- **La 3060 refuse les surfaces X11** dans la session de la machine : le jeu doit compiler le support Wayland de winit.
- **`bevy_render` inverse la matrice de la caméra en f32** (`Mat4::inverse`) : loin de l'origine, l'erreur dépasse le simple arrondi des positions (témoin de Brest : 12 cm pour un pas de 3 cm).
- **`big_space` sans `Stationary`** repropage chaque modèle à chaque frame.

### Limites

- **Scène synthétique** : capsules, blocs, sol plat (pas de relief RGE ALTI), globe sans texture ni continents ; pas de bâtiments réels (P-08), pas de foules instanciées (P-11).
- **Pas de transition sol → globe** : le globe est simplement sous le sol des tiles (à −2 m) ; le choix du niveau de détail relève de C-02, C-03 et P-10.
- **Temps GPU non mesuré** : la cadence hors écran mesure le temps de frame complet, mais rien ne garantit que le CPU attende le GPU ; à 1 million, le GPU peut être le facteur limitant sans que cela se voie.
- **Ombres** : comparaison d'images fixes ; le scintillement en mouvement n'est pas mesuré ; une seule configuration de cascades.
- **Horloge des shaders** : calculée sur le CPU en f32 comme le ferait un shader (le bassin rayé de `voir.sh` l'anime par `uv_transform`) ; pas de shader WESL écrit.
- **Projections officielles** : les codes EPSG sont ceux de PROJ ; le fuseau d'usage de chaque archipel est à vérifier ; les TAAF et Clipperton sont exclus du jeu.
- **Marche A** : sur la sphère, pas sur l'ellipsoïde ; son coût réel serait plus élevé.
- **Déterminisme entre plateformes** non testé (Linux seulement, P-18).

## Conclusion

- **C-01 est résolu** par le rendu relatif à la caméra : écart sous 0,02 px (0,1 mm) à Brest, Tahiti et au point le plus éloigné de Paris (île Walpole, 16 866 km), confirmé par le rendu réel (sous le plancher de 0,1 px des captures). Les témoins naïfs tremblent de 8 à 300 px.
- **Simulation : tile + position locale f32 centrée** (C). 8 octets par position, le plus rapide, invariant par translation, passages de tile exacts ; une zone de projection par territoire (plusieurs pour la Polynésie), ECEF f64 en mer seulement.
- **Repère de tile au second ordre** : 0,005 mm d'écart, 0,0 mm de saut aux changements de zone ; le plan tangent est faux de 39 mm.
- **Recalage au-delà d'un seuil de quelques kilomètres**, fait par le jeu, sans hiérarchie et sans `big_space` ; jusqu'à 10 km, l'écart reste sous 0,15 px. À chaque frame, il coûte 8,6 ms de frame à 100 000 modèles et fait trembler les grands maillages.
- **Globe découpé** en morceaux à origine locale ; sol des tiles en dessous de 1 à 10 km.
- **Profondeur** : rien à faire. **Ombres** : un saut de 0,2 à 0,5 % des pixels de bord à chaque recalage.
- **Temps des shaders** : phases calculées depuis les ticks entiers, jamais un f32 absolu (visible après 36 h de jeu) ; ticks en u64 partout.
- **Rendu** : le nombre d'entités rendues est la vraie limite (22 ms par frame à 1 million) ; les foules passeront par l'instanciation.

## Décisions

Prises par l'utilisateur sur les recommandations de ce prototype, et reportées dans [OPEN-PROBLEMS.md](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-01--précision-numérique) (C-01, C-02, C-03, C-05, A-01, A-02, A-13, F-02, G-01), [SYSTEMES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/systemes/SYSTEMES.md#repères-et-précision), le [README](https://github.com/labatailledefrance/documentation/blob/main/README.md) du jeu et `CLAUDE.md`.

- **Repère de la simulation** : tile entière + position locale **f32 centrée** sur la tile (±500 m) ; tile = (zone de projection, ix, iy). Les navires en mer sont en **ECEF f64**.
- **Zones de projection** : Lambert-93 en métropole (Corse comprise) ; UTM ou Lambert local pour chaque territoire, une zone par fuseau en Polynésie (UTM 5S à 8S). Projection d'usage de chaque territoire toujours à vérifier.
- **Périmètre** : **pas de TAAF** (Kerguelen, Crozet, Amsterdam et Saint-Paul, îles Éparses, Terre Adélie) ni de **Clipperton** : inhabitables.
- **Repère d'une tile** : au **second ordre** (Taylor au centre), pas en plan tangent.
- **Changement de zone d'un navire** : par l'ECEF, à 30 km des côtes (valeur de départ).
- **Recalage du rendu** : au-delà d'un **seuil de 5 km** entre la caméra et l'origine, écrit dans le jeu ; ni `big_space`, ni hiérarchie de Bevy pour les modèles.
- **Globe** : découpé en morceaux à origine locale ; le sol des tiles est affiché sous 1 à 10 km d'altitude de caméra (seuil exact : C-02, P-10).
- **Sauvegarde de partie** : dans les tiles actives, positions en **f32 exacts** (environ 6 octets), pour qu'un rechargement reproduise exactement la partie continuée ; les tiles inertes restent au 1/64 m (A-13, P-04).
- **Ticks en u64** partout ; **temps des shaders** : uniquement des phases calculées sur le CPU depuis les ticks entiers.
- **Dates relatives compactes** : la date de dernière mise à jour d'un colon (P-05) reste un u32, comparée par soustraction modulaire (`wrapping_sub`), comme les numéros de séquence TCP ; la fiche garde ses 32 octets. Juste tant qu'un colon est mis à jour au moins une fois tous les 6,8 ans.
- **Rendu** : foules et modèles nombreux par **instanciation** (tampon de positions relatives à l'origine) ; environ 100 000 entités rendues au plus.
- **Ombres** : caler les cascades sur leur grille de texels, à étudier dans P-10.

## Questions ouvertes

Aucune pour l'instant : les questions de ce prototype et celles qu'ont soulevées ses décisions sont toutes tranchées.
