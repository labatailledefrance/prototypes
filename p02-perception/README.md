# P-02 — Perception des zombies

Prototype jetable. Problème : [A-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-01--nombre-de-zombies--simulation-par-tiles). Fiche du prototype : [prototypes/README.md](../README.md#p-02--perception-des-zombies). Point de départ : [P-01](../p01-tile-paris/README.md).

## Hypothèse testée

Dans P-01, la perception est le premier poste du tick : 45 % d'une tile calme, jusqu'à 75 % en combat sans relais. La grille uniforme de 25 m qu'il utilise est naïve : un zombie examine 289 cases pour une vue de 200 m, et en assaut chaque poursuivant parcourt les 1 000 humains du groupe.

Une structure de données mieux adaptée à la question « quels humains sont dans mon rayon d'ouïe et mon cône de vue, sans bâtiment entre nous ? » réduit-elle ce coût d'un ordre de grandeur ? Laquelle retenir, avec des humains **mobiles** (index reconstruit à chaque tick) ? Combien de tiles deviennent alors tenables à ×1 et à ×10 ?

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, Linux 7.0. Rust 1.99, `bevy_ecs` 0.20.1. Mesures du 2026-11-14.

## Méthode

### Point de départ

La simulation est celle du test 2 de P-01 : emprises réelles de la BD TOPO (`data/generated/p01-tiles/`), 20 000 zombies et 1 000 humains par tile, relais d'alerte, veille, champ de distance, capacité par case. [src/ville.rs](src/ville.rs), [src/grille.rs](src/grille.rs), [src/alea.rs](src/alea.rs) et [src/mesure.rs](src/mesure.rs) sont copiés de P-01, qui n'est pas modifié. Les agents sont placés dans la plus grande zone de rues connexe, comme dans P-01.

Les décisions de [ZOMBIES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/zombies/ZOMBIES.md#perception), de F-01 et de F-02 sont appliquées : perception à 4 Hz, veille à 1 Hz, vue de 200 m, relais valable 3 s, dernière position suivie pendant une minute, `par_iter_mut` avec un lot minimal de 1 024, composants groupés, états en champs.

### La requête

Une perception pose deux questions à l'index des humains de la tile :

- **ouïe** : l'humain le plus proche à moins de 40 m, dans toutes les directions ;
- **vue** : les 4 humains les plus proches dans le cône de 120°, entre 40 et 200 m. Ce sont les candidats au tracé de ligne de vue sur le bitmap de 1 m.

Un humain entendu l'emporte toujours sur un humain vu, puisqu'il est plus près. Les structures nouvelles traitent donc l'ouïe d'abord, et ne cherchent dans le cône que si personne n'est entendu. C'est la même règle que P-01, appliquée dans un ordre qui permet de s'arrêter plus tôt.

### Les structures comparées

[src/index.rs](src/index.rs) en propose quatre, interchangeables (`--structure`) :

| Structure | Principe | Analogie |
|---|---|---|
| **Grille** (P-01) | Cases uniformes ; toutes les cases du carré de 400 m de côté, en un seul passage | Un parcours de plage complet, sans index trié |
| **Anneaux** | Même grille, parcourue par anneaux de distance croissante ; cases hors du disque ou hors du cône sautées ; arrêt dès qu'un anneau est plus loin que le résultat courant | Un index trié avec `LIMIT k` |
| **Hiérarchique** | Cases grossières (8 × la case fine) qui portent le nombre d'humains, triées par distance ; cases fines seulement dans les cases grossières non vides | Une table des partitions avant les partitions |
| **Arbre k-d** | Coupes successives à la médiane sur l'axe le plus étendu, feuilles de 8 humains, boîtes englobantes ajustées : une BVH de points. Parcours du nœud le plus proche au plus loin, élagage par distance et par cône | Un B-tree spatial |

Deux options s'ajoutent :

- **Rejet rapide** (`--rejet`) : une **carte de proximité** à cases de 25 m donne, en O(1), une borne inférieure de la distance au plus proche humain. On la recalcule à chaque tick par deux balayages, avec la distance de Tchebychev, comme une table de routage « distance au préfixe le plus proche ». Si aucun humain n'est à portée, on ne pose pas de requête ; si aucun n'est à portée d'ouïe, on saute la phase d'ouïe.
- **Filtre du relais** (`--filtre-relais`) : en reconstruisant la grille des zombies, on pose un bit par case de 2 m qui contient un zombie porteur d'alerte. Un zombie dont les 9 cases voisines n'ont aucun bit ne lit pas leurs occupants.

Il n'y a **pas de cache des lignes de vue** : le banc montre qu'une ligne de vue coûte 80 à 150 ns, soit le quart d'une requête avec l'arbre k-d (voir l'analyse). Un cache, invalidé à chaque pas d'un humain ou d'un zombie, ne rapporterait presque rien.

### Ce que P-02 ajoute à la simulation

- **Humains mobiles** : marche aléatoire à 1,5 m/s dans les rues accessibles (dans le disque du groupe pour les scénarios calme et assaut). L'index et la carte de proximité sont **reconstruits à chaque tick**, dans une nouvelle phase « humains », chronométrée. `--immobiles` revient au comportement de P-01.
- **Veille profonde** (`--profonde`, décision de A-01) : un zombie en errance dont la carte de proximité garantit qu'aucun humain n'est à moins de 300 m passe à l'état « profond ». Il sort de l'index des zombies, ne lit plus de relais et se déplace une fois par seconde, sans séparation ni capacité. Une fois par seconde, il teste en O(1) la carte de proximité et les bruits, et se réveille si un humain approche ou si un bruit porte jusqu'à lui.
- **Compteurs de travail** par thread, sans contention, comme les compteurs `percpu` du noyau : requêtes, rejets, cases parcourues, humains examinés, lignes de vue tracées.

### Déterminisme

Toutes les structures rendent **exactement** la même cible : les égalités de distance sont départagées par l'identifiant de l'humain. Le résultat ne dépend donc ni de la structure, ni du rejet, ni du filtre, ni du nombre de threads, ni du découpage en lots. C'est vérifié :

- par les tests (`cargo test --release`, 9 tests) : `structures_equivalentes` compare toutes les structures, à six tailles de case, à la grille de P-01, sur 6 000 requêtes, doublons compris ; `independant_de_la_structure`, `independant_du_decoupage` et `bascule_de_structure` comparent les sommes de contrôle de simulations complètes ;
- par le banc, qui refuse une structure dont l'empreinte diffère de la référence ;
- par la colonne `somme` de [resultats.csv](resultats.csv) : à configuration de comportement égale, elle est identique pour toutes les structures et pour 1 à 16 threads.

Cette équivalence permet de **préchauffer** avec l'arbre k-d, le plus rapide, puis de basculer sur la structure mesurée (`--chauffe`).

### Deux mesures

- **Banc** (`--banc`) : après le préchauffage, les 20 000 zombies de la tile posent chacun une requête à chaque structure, sur le même instantané, sur **un seul thread**. On obtient le coût pur d'une requête, ligne de vue comprise, et le coût de reconstruction de l'index. Résultats : [resultats-banc.csv](resultats-banc.csv).
- **Simulation** : tick complet, 16 threads, comme dans P-01. Médiane et p99 sur 200 ticks. Résultats : [resultats.csv](resultats.csv).

Scénarios calme, dispersé et assaut de P-01. L'assaut est mesuré après 120 s de jeu, les autres après 10 s.

Reproduire : `./run.sh`, environ 35 min (`SANS_BANC=1 ./run.sh` pour sauter le banc). Le script reconstruit le binaire release avant de mesurer, puisque `cargo test` ne le fait pas.

### Paramètres de prototype

Ceux de P-01, plus : marche des humains à 1,5 m/s, carte de proximité à cases de 25 m, seuil de veille profonde à 300 m. Ce sont des hypothèses de mesure, pas des statistiques de jeu.

## Budgets de référence

Les mêmes que pour P-00 et P-01 : 25 ms par tick à ×1, 2,5 ms à ×10.

## Mesures

### Banc : coût d'une requête, un thread

En ns par requête, ligne de vue comprise. Instantané avec relais et veille, une tile réelle.

| Structure | Case | Calme | Dispersé | Assaut |
|---|---|---|---|---|
| Grille (P-01) | 10 m | 2 954 | 4 143 | 6 205 |
| | **25 m (référence)** | **1 333** | **2 237** | **4 373** |
| | 50 m | 1 289 | 1 590 | 5 121 |
| | 100 m | 1 378 | 1 616 | 5 471 |
| Anneaux | 5 m | 36 365 | 153 | 8 330 |
| | 10 m | 9 910 | 96 | 2 541 |
| | 25 m | 2 066 | **79** | 885 |
| | 50 m | 885 | 87 | 1 021 |
| Hiérarchique | 6,25 m | 381 | 265 | 706 |
| | 12,5 m | 350 | 271 | 650 |
| | 25 m | 402 | 303 | 946 |
| **Arbre k-d** | — | **173** | 228 | **382** |
| Arbre k-d + rejet | — | 158 | 244 | 390 |
| Anneaux 10 m + rejet | — | 3 378 | 101 | 2 082 |

Sans relais, les chiffres sont à 10 % près les mêmes ([resultats-banc.csv](resultats-banc.csv)).

Travail moyen par requête, avec la même structure de colonnes :

| Structure | Cases ou nœuds parcourus (calme / dispersé / assaut) | Humains examinés (calme / dispersé / assaut) |
|---|---|---|
| Grille 25 m | 236 / 236 / 283 | 237 / 160 / 945 |
| Anneaux 25 m | 180 / 1,5 / 43 | 13,5 / 2,2 / 69 |
| Hiérarchique 12,5 m | 52 / 71 / 112 | 14,5 / 3,6 / 60 |
| Arbre k-d | 2,3 / 8,2 / 9,3 | 3,1 / 8,2 / 11,6 |

Reconstruction de l'index, 1 000 humains : 6 µs pour une grille, 10 µs pour la grille hiérarchique de 12,5 m, **46 µs pour l'arbre k-d**.

Lignes de vue : 0,38 par requête au calme, 0,01 en dispersé (presque tout le monde entend un humain), 1,06 en assaut. Chacune coûte 80 à 150 ns.

En requêtes par seconde et par thread : 0,23 à 0,75 million pour la grille de P-01, **2,6 à 5,8 millions pour l'arbre k-d**.

### Simulation : phase de perception, 16 tiles

En ms par tick (médiane), 16 tiles réelles, 16 threads. C'est la seule phase qui dépend de la structure.

| Structure | Calme | Dispersé | Assaut | Calme, R+V | Dispersé, R+V | Assaut, R+V |
|---|---|---|---|---|---|---|
| Grille 25 m (P-01) | 7,66 | 12,4 | 27,1 | 4,90 | 5,83 | 17,0 |
| Grille 50 m | 6,82 | 8,44 | 27,9 | 4,58 | 4,99 | 16,6 |
| Anneaux 25 m | 15,8 | **1,13** | 9,34 | 6,61 | 3,61 | 10,5 |
| Hiérarchique 12,5 m | 2,33 | 2,46 | 5,80 | 3,42 | 3,91 | 6,44 |
| **Arbre k-d** | **1,38** | 1,79 | **4,57** | **3,10** | 3,77 | **4,71** |
| Arbre k-d + rejet | 1,36 | 1,80 | 4,00 | 3,24 | 3,76 | 4,76 |

R+V : relais et veille. Sans relais, chaque zombie actif perçoit 4 fois par seconde (64 000 requêtes par tick) ; avec relais et veille, 11 500 à 25 600.

### Simulation : étapes de la configuration retenue, 16 tiles

Tick complet, médiane (p99), en ms. Toutes avec l'arbre k-d et le rejet.

| Configuration | Calme | Dispersé | Assaut |
|---|---|---|---|
| Référence P-01 : grille 25 m, relais, veille | 10,5 (14,4) | 15,1 (20,4) | 26,7 (32,3) |
| Arbre k-d, veille, **sans relais** | 7,33 (10,0) | **11,1 (15,6)** | **11,3 (15,4)** |
| Arbre k-d, relais, veille | 8,86 (17,7) | 12,8 (18,0) | 12,9 (18,9) |
| + filtre du relais | 8,21 (10,9) | 13,2 (17,2) | 12,8 (17,7) |
| **+ veille profonde (retenue)** | **6,31 (9,02)** | 13,1 (19,3) | 12,3 (17,6) |

Répartition de la configuration retenue, 16 tiles, en ms :

| Scénario | Humains | Grilles | Perception | Déplacement |
|---|---|---|---|---|
| Calme | 0,32 | 2,84 | 1,36 | 1,74 |
| Dispersé | 0,42 | 2,94 | 3,97 | 5,73 |
| Assaut | 0,33 | 2,82 | 4,35 | 4,76 |

Au calme, la veille profonde endort 126 000 des 320 000 zombies (39 %).

### Montée en charge

Médiane (p99), en ms. « Réf. » : grille de 25 m, relais et veille (P-01, humains mobiles). « Retenue » : arbre k-d, rejet, relais filtré, veille, veille profonde.

| Tiles | Calme, réf. | Calme, retenue | Dispersé, réf. | Dispersé, retenue | Assaut, réf. | Assaut, retenue |
|---|---|---|---|---|---|---|
| 1 | 1,25 (1,42) | 0,97 (1,01) | 1,47 (1,65) | 1,42 (1,66) | 1,81 (2,34) | 1,28 (1,30) |
| 2 | 1,67 (2,06) | 1,21 (1,27) | 2,22 (2,76) | 2,05 (2,74) | 3,02 (4,31) | 1,83 (2,36) |
| 4 | 2,53 (2,87) | 1,72 (2,17) | 3,87 (4,89) | 3,54 (5,09) | 6,28 (8,09) | 3,20 (4,18) |
| 8 | 4,47 (6,68) | 2,52 (3,38) | 7,03 (13,9) | 6,18 (9,55) | 11,9 (16,1) | 5,73 (8,11) |
| 16 | 10,5 (14,4) | 6,31 (9,02) | 15,1 (20,4) | 13,1 (19,3) | 26,7 (32,3) | 12,3 (17,6) |
| 24 | 17,8 (23,0) | 10,9 (13,0) | 23,6 (32,3) | 21,3 (28,0) | 37,6 (46,2) | 19,0 (24,1) |
| 32 | 27,0 (33,7) | 14,9 (18,2) | 33,0 (43,1) | 29,0 (36,6) | 46,6 (54,2) | 25,4 (32,0) |
| 48 | 39,1 (49,5) | 22,5 (26,6) | | | | |
| 64 | 51,2 (59,6) | 31,0 (40,1) | | | | |

Répartition, calme, 64 tiles, configuration retenue : humains 0,9 ms (3 %), **grilles 14,0 ms (45 %)**, perception 6,7 ms (22 %), déplacement 9,2 ms (30 %).

### Distance de vue, 16 tiles, sans relais

Phase de perception, ms :

| Vue | Dispersé, grille 25 m | Dispersé, arbre k-d | Assaut, grille 25 m | Assaut, arbre k-d |
|---|---|---|---|---|
| 100 m | 5,13 | 1,81 | 22,0 | 2,45 |
| 200 m | 12,4 | 1,80 | 27,1 | 4,00 |
| 400 m | 29,4 | 1,81 | 34,9 | 3,41 |

### Humains mobiles

Phase « humains », 16 tiles : 0,22 ms avec la grille, **0,32 à 0,42 ms avec l'arbre k-d**, contre 0,016 ms avec des humains immobiles. Sur une tile seule, c'est 0,12 ms, en série. Le tick de la configuration retenue passe de 6,00 à 6,31 ms au calme (16 tiles) quand les humains bougent.

### Threads, dispersé, 16 tiles, configuration retenue

| Threads | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| Tick (ms) | 125 | 63,4 | 33,0 | 17,8 | 13,1 |
| Accélération | ×1 | ×2,0 | ×3,8 | ×7,0 | ×9,5 |

La somme de contrôle est identique sur les cinq lignes.

### Mémoire et création

Inchangées depuis P-01 : environ 8 Mo et 37 ms par tile. L'arbre k-d d'une tile (1 000 humains) pèse environ 20 ko.

## Analyse

**1. L'arbre k-d divise le coût d'une requête par 8 à 13.** Sur le banc, il passe de 1 300 à 170 ns au calme, de 2 200 à 230 ns en dispersé et de 4 400 à 380 ns en assaut. Il est premier au calme et en assaut, et deuxième en dispersé, derrière les anneaux. C'est la seule structure qui n'a **aucun cas pathologique** : il parcourt 2 à 9 nœuds et examine 3 à 12 humains, quelle que soit la répartition. La raison est la même que pour un B-tree : ses boîtes s'ajustent aux données. Un groupe dense est découpé finement, l'espace vide tient dans une seule boîte rejetée d'un coup.

**2. Les grilles ont toutes un point faible, qui dépend de la taille des cases.** Petites cases : on parcourt des centaines de cases vides dès que les humains sont loin (anneaux de 10 m au calme : 1 019 cases, 10 µs). Grandes cases : on examine des centaines d'humains dès qu'ils sont groupés (grille de 100 m en assaut : 960 humains). Aucune taille ne convient à la fois au calme, au dispersé et à l'assaut. Les anneaux sont imbattables en dispersé (79 ns), mais 12 fois plus lents que l'arbre au calme. La grille hiérarchique corrige les deux défauts. Elle est robuste, mais 1,5 à 2 fois plus lente que l'arbre.

**3. Le coût ne croît plus avec le carré de la distance de vue.** Avec la grille, passer de 100 à 400 m multiplie la perception par 6 en dispersé. Avec l'arbre, elle reste à 1,8 ms : on s'arrête aux 4 candidats les plus proches, quelle que soit la portée. La justification du plafond de vue de 100 à 200 m (« son coût croît avec son carré », P-01) ne tient plus : le plafond a été relevé (voir Décisions).

**4. Le rejet rapide est inutile avec l'arbre.** L'arbre élague déjà l'espace vide à la racine : le rejet ne gagne rien en simulation, et 8 % au plus sur le banc. La carte de proximité reste indispensable, mais pour la **veille profonde**, qui a besoin d'un test en O(1) pour décider qui dort et qui se réveille.

**5. La reconstruction de l'index à chaque tick est abordable.** L'arbre se construit en 46 µs pour 1 000 humains, 8 fois plus lentement qu'une grille, mais il se rembourse dès quelques centaines de requêtes : une tile en pose 700 à 4 000 par tick. Le surcoût des humains mobiles est de 0,3 ms pour 16 tiles, 2 à 5 % du tick. Cela suppose un millier d'humains par tile. Une colonie de 10 000 civils individualisés coûterait 0,5 ms par tile et par tick, en série : à surveiller avec P-05.

**6. La perception n'est plus le premier poste, et c'est le relais qui coûte le plus dans ce qui reste.** Avec l'arbre, une requête coûte 0,2 à 0,4 µs ; une tile en pose quelques milliers par tick, soit moins de 1 ms de CPU, moins de 0,1 ms sur 16 threads. Le reste de la phase de perception est la recherche du meilleur relais : chaque zombie qui n'a pas vu la cible lit jusqu'à 16 voisins **à chaque tick**. Conséquence surprenante : **sans relais, le tick est plus court qu'avec** (assaut, 16 tiles : 11,3 ms contre 12,8 ; dispersé : 11,1 contre 13,2). Le relais a été adopté dans P-01 pour son gain de calcul ; avec un bon index, ce gain disparaît. Il reste une règle de jeu, celle des hordes émergentes, qui coûte 10 à 20 % du tick en combat.

**7. Le filtre du relais règle le cas calme, pas le combat.** Au calme, la plupart des zombies errent loin de toute alerte : le filtre évite leur recherche, et la phase de perception passe de 3,24 à 1,63 ms (16 tiles). En combat, presque tous les zombies sont au contact d'une alerte et doivent lire leurs voisins : aucun gain. Pour réduire le coût en combat, il faudrait relayer moins souvent, ce qui ralentirait la vague (voir les questions).

**8. La veille profonde gagne 23 % au calme, et son reliquat est dans la grille des zombies.** 39 % des zombies d'une tile calme dorment profondément ; le tick passe de 8,2 à 6,3 ms (16 tiles). Mais la reconstruction de la grille des zombies ne baisse que de 22 % (3,6 → 2,8 ms), alors qu'il y a 39 % de zombies en moins à indexer. Son coût vient surtout de ses 250 000 cases de 2 m (remise à zéro, sommes cumulées), pas des zombies. C'est désormais **le premier poste au calme** : 45 % du tick à 64 tiles.

**9. Tiles tenables** (lu sur le p99, configuration retenue) :

| Scénario | ×1 (25 ms) | ×10 (2,5 ms) | Référence P-01 à ×1 | Référence P-01 à ×10 |
|---|---|---|---|---|
| Calme | **≈ 44** | **4** | ≈ 24 | 2 |
| Dispersé | ≈ 20 | 1 | ≈ 16 à 18 | 1 |
| Assaut | **≈ 24** | **2** | ≈ 12 | 1 |

Coût marginal d'une tile : environ **0,48 ms** au calme (contre 0,80 pour la référence), **0,90 ms** en dispersé (1,03) et **0,79 ms** en assaut (1,46). L'assaut, pire cas de P-01, double sa capacité : la foule ne coûte plus que ce que coûte le déplacement. Le dispersé gagne peu. Ses 20 000 zombies sont tous en poursuite ou relayés : le déplacement (44 %) et la recherche de relais dominent, pas les requêtes.

**10. Passage à l'échelle : ×9,5 sur 16 threads**, contre ×11 dans P-01. Le travail restant est plus léger par zombie, et les phases en série (une tâche par tile) pèsent un peu plus.

### Limites

- **Pas de frontière entre tiles** : un zombie ne perçoit pas les humains de la tile voisine. Avec une vue de 200 m, un index par tile ne suffira pas (voir les questions).
- **Humains en marche aléatoire** : ni tir, ni fuite, ni infection, ni véhicules. Leur répartition reste celle des scénarios de P-01.
- **Un millier d'humains par tile.** Une colonie dense ou une base de plusieurs milliers de soldats changerait le coût de reconstruction, pas le choix de la structure (l'arbre est en O(log n) par requête).
- **Bruit de mesure** : la phase de déplacement, qui ne dépend pas de la structure, varie de 4,8 à 7,5 ms d'une mesure à l'autre en assaut à 16 tiles (arbre k-d sans relais, mesure refaite deux fois). Les conclusions portent sur la phase de perception, stable, et sur des écarts bien plus grands que ce bruit.
- **Requête mono-tile** : le banc mesure le coût pur sur un thread. Dans la simulation, le gain se dilue dans le reste de la phase de perception (machine à états, relais, publication des alertes).
- **Pas d'étages** : la vue et l'ouïe restent en 2D, comme dans P-01.

## Complément : distance de vue et veille profonde

Après la décision de relever le plafond de vue, la veille profonde commence au-delà de la vue (seuil : vue + 100 m). [run-vue.sh](run-vue.sh) mesure la configuration retenue à 200, 400 et 800 m ([resultats-vue.csv](resultats-vue.csv)).

Tick, 16 tiles, médiane (p99), en ms :

| Vue | Calme | Dispersé | Assaut | Zombies en veille profonde (calme) |
|---|---|---|---|---|
| 200 m | 6,79 (9,49) | 15,0 (20,5) | 12,6 (16,7) | 126 000 (39 %) |
| 400 m | 9,04 (11,1) | 13,5 (19,5) | 13,2 (17,9) | 0 |
| 800 m | 10,8 (13,9) | 14,2 (22,7) | 14,6 (21,1) | 0 |

- **Au calme, la vue coûte de nouveau, mais indirectement** : +33 % à 400 m, +59 % à 800 m. Ce n'est pas la requête (l'arbre ne dépend pas de la portée) : dès 400 m, plus aucun zombie de la tile ne dort profondément. Les zombies qui voient le groupe de plus loin se mettent aussi en poursuite : il y a davantage de lignes de vue tracées (0,5 → 1,7 par requête) et de zombies qui courent.
- **En combat, l'effet est faible** : +5 % à 400 m, +16 % à 800 m en assaut ; rien en dispersé, où tout le monde poursuit déjà.
- **Tiles tenables à ×1**, extrapolées linéairement depuis le p99 à 16 tiles : au calme, environ 36 à 400 m et 28 à 800 m (44 à 200 m) ; en assaut, environ 22 et 19 (24). Même à 800 m, c'est mieux que P-01 (24 et 12).
- **Mesure pessimiste** : une tile isolée ne compte que ses propres humains, au centre. Dans le jeu, les zombies des tiles plus éloignées du front dormiraient encore, et le surcoût serait limité aux tiles du front.

## Conclusion

- **Structure retenue : un arbre k-d par tile** (BVH de points : médiane sur l'axe le plus étendu, feuilles de 8, boîtes ajustées), **reconstruit à chaque tick**, parcouru du plus proche au plus loin, **ouïe d'abord, puis vue**. Il divise le coût d'une requête par 8 à 13, n'a aucun cas pathologique, et rend la perception indépendante de la distance de vue.
- **Écartées** : la grille de P-01, à toutes les tailles de case (point faible garanti) ; les anneaux (12 fois plus lents que l'arbre au calme) ; la grille hiérarchique, robuste mais 1,5 à 2 fois plus lente que l'arbre, sans avantage en échange ; le rejet rapide comme filtre de perception ; le cache des lignes de vue.
- **À garder** : la **carte de proximité**, pour la veille profonde ; le **filtre du relais** (−50 % de perception au calme, exact, presque gratuit) ; la **veille profonde** (−23 % au calme).
- **Avec la configuration retenue, environ 44 tiles tenables au calme, 20 en dispersé et 24 en assaut à ×1 ; 4, 1 et 2 à ×10.** C'est à peu près le double de P-01 au calme et en assaut.
- **Les postes suivants ne sont plus la perception** : la reconstruction de la grille des zombies (45 % au calme), le déplacement, et la recherche de relais en combat.

## Décisions

Prises à l'issue du prototype et reportées dans les documents de conception :

- **Structure de perception** : arbre k-d des humains par tile, reconstruit à chaque tick ([A-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-01--nombre-de-zombies--simulation-par-tiles), [SYSTEMES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/systemes/SYSTEMES.md#simulation-par-tiles)).
- **Distance de vue** : plus de plafond de coût ; environ 400 m en ville, jusqu'à 800 m en terrain découvert, toujours sous la largeur de la couronne ; **fixée par tile**, selon sa part de bâti calculée à la préparation de la carte ([ZOMBIES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/zombies/ZOMBIES.md#perception)). En conséquence, la veille profonde commence au-delà de la distance de vue.
- **Index des humains denses** : reconstruit environ 4 fois par seconde pour une tile de plusieurs milliers d'humains individualisés.
- **Relais** : recherche à chaque tick, pour garder une portée d'environ 150 m ([ZOMBIES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/zombies/ZOMBIES.md#perception)).
- **Perception entre tiles** : un arbre par tile ; un zombie interroge aussi ceux des tiles voisines que sa vue atteint.
- **Grille des zombies** : son remplacement est confié à [P-03](../README.md#p-03--flow-fields).

## Questions ouvertes

- (aucune : les décisions sont reportées dans les documents de conception ; la suite est [P-03](../README.md#p-03--flow-fields))
