# P-01 — Coût d'une tile parisienne

Prototype jetable. Problèmes : [A-01](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-01--nombre-de-zombies--simulation-par-tiles), [A-15](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-15--civils-en-métropole--colonies). Fiche du prototype : [prototypes/README.md](../README.md#p-01--coût-dune-tile-parisienne).

## Hypothèse testée

Une tile active de 1 km² à densité parisienne (20 000 zombies agents, bâti dense) tient-elle dans le budget de simulation ? Combien de tiles de ce type peut-on simuler en même temps, à ×1 et à ×10 ?

Deux tests :

- **test 1** : emprises **synthétiques**, perception individuelle, déplacement en ligne droite ;
- **test 2** ([plus bas](#test-2--relais-veille-et-emprises-réelles)) : **relais d'alerte**, **mise en veille** et **emprises réelles** de la BD TOPO, avec un champ de distance pour le déplacement.

Les sections Méthode, Mesures et Analyse qui suivent décrivent le test 1 ; le test 2 n'en reprend que les différences.

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, Linux 7.0. Rust 1.99, `bevy_ecs` 0.20.1. Mesures du 2026-11-14.

## Méthode

### Décisions appliquées

- `bevy_ecs` avec l'exécuteur multi-thread et des requêtes `par_iter_mut` (F-01).
- Composants groupés selon le système qui les écrit : `Corps` (position, cap), écrit par le déplacement ; `Cerveau` (état, cible), écrit par la perception ; `Ident` (tile, rang), jamais modifié (F-02).
- Détection de changement contournée : ces données changent à chaque tick (P-00).
- État du zombie (errance, poursuite, attiré par un bruit) porté par un **champ**, pas par un marqueur (F-02).
- Taille de lot minimale sur toutes les requêtes parallèles (F-02).
- Positions en Lambert-93 sur 64 bits (piste de C-01). Les calculs se font en 32 bits, en coordonnées locales à la tile.

### La ville

[src/ville.rs](src/ville.rs) génère une tile de 1 km × 1 km découpée en îlots haussmanniens :

- rues de 8 à 20 m, quelques boulevards de 30 à 40 m ;
- îlots de 50 à 140 m de côté, 5 % de places ;
- deux avenues diagonales de 25 m qui se croisent au centre.

Elle donne 52 à 57 % de surface bâtie, l'ordre de grandeur du Paris dense. Le bâti est stocké comme un bitmap de 1 m : un bit par m², 125 ko par tile. Analogie : le bitmap d'allocation des blocs d'un système de fichiers. Il tient dans le cache L2 d'un cœur.

### Les agents

- **20 000 zombies par tile**, tirés au hasard dans les rues. Personne n'est à l'intérieur des bâtiments (c'est le sujet de P-04).
- **1 000 humains par tile**, immobiles. Ils ne servent que de cibles : pas de tir, pas d'infection.

### Un tick, en trois phases chronométrées

1. **Grilles** ([src/grille.rs](src/grille.rs)) : chaque tile reconstruit l'index « case de 2 m → zombies » par un tri par dénombrement, à partir des positions du tick précédent. Une tâche par tile.
2. **Perception** : chaque zombie perçoit un tick sur 5, soit 4 fois par seconde, de façon étalée entre les zombies. Il cherche dans une grille d'humains à cases de 25 m :
   - l'**ouïe** : l'humain le plus proche dans le rayon d'ouïe, dans toutes les directions et à travers les murs ;
   - la **vue** : les 4 humains les plus proches dans le cône et à distance de vue, avec un tracé de ligne de vue case par case sur le bitmap (algorithme DDA, l'équivalent d'un traceroute qui s'arrête au premier mur) ;
   - les **bruits**, s'il n'y a pas d'humain.

   Au test 1, sans stimulus, le zombie perd la trace immédiatement. Le test 2 applique les décisions prises depuis : relais, dernière position connue ([ZOMBIES.md](https://github.com/fafapiti/documentation/blob/main/game-elements/zombies/ZOMBIES.md#perception)).
3. **Déplacement** : en ligne droite vers la cible (pas de flow field, c'est P-03), ou en errance. Les collisions restent simples :
   - séparation entre zombies, sur 16 voisins au plus ;
   - glissement le long des murs ;
   - **capacité par case** de 2 m × 2 m, avec une admission progressive : une case à moitié pleine ne laisse entrer qu'un candidat sur deux.

Chaque phase lit l'état du tick précédent et n'écrit que l'entité qu'elle traite. Le résultat ne dépend donc ni du nombre de threads ni du découpage en lots. C'est vérifié par un test, et par la somme de contrôle de chaque ligne des résultats.

### Paramètres de prototype

Ce sont des **hypothèses de mesure, pas des statistiques de jeu** :

- course 5 m/s, errance 1 m/s ;
- ouïe 40 m ;
- vue 200 m, cône de 120° ;
- séparation 0,7 m ;
- capacité de 24 zombies par case de 4 m² (6 par m²) ;
- bruit de tirs entendu à 600 m.

### Scénarios

| Scénario | Humains | Bruit | Ce qu'il représente |
|---|---|---|---|
| Calme | 1 000, groupés dans un disque de 60 m au centre | aucun | une base silencieuse : seuls les zombies proches réagissent |
| Assaut | idem | tirs continus au centre | la base se défend : toute la tile converge |
| Dispersé | 1 000, répartis dans toute la tile | aucun | des patrouilles partout : pire cas pour la perception |

L'assaut est mesuré après 180 s de jeu (3 600 ticks), quand la foule est formée. Les autres scénarios sont mesurés après 10 s. Chaque mesure porte sur 200 ticks et donne la médiane et le p99.

Reproduire : `./run.sh`. Il écrit [resultats.csv](resultats.csv) et [resultats-chronologie.csv](resultats-chronologie.csv). Tests : `cargo test --release`.

Le code actuel est celui du test 2 : `run.sh` le lance avec ses réglages par défaut (relais et veille désactivés), mais le déplacement suit désormais le champ de distance, et les zombies publient leurs alertes. Les temps de base sont 5 à 10 % plus élevés qu'au test 1 ; les comparaisons du test 2 sont donc toutes faites avec le même binaire.

## Budgets de référence

Les mêmes que pour P-00 : 25 ms par tick à ×1, 2,5 ms à ×10.

## Mesures (test 1)

Temps par tick en ms, 16 threads, taille de lot minimale de 1 024. On donne la **médiane (p99)**.

### Montée en charge : nombre de tiles actives

| Tiles | Calme | Dispersé | Assaut (après 180 s) |
|---|---|---|---|
| 1 | 1,24 (3,02) | 1,57 (2,10) | 2,17 (3,29) |
| 2 | 1,82 (2,42) | 2,68 (4,16) | 3,82 (5,35) |
| 4 | 3,36 (4,04) | 4,94 (7,04) | 7,23 (10,0) |
| 8 | 5,94 (8,08) | 9,22 (13,6) | 13,9 (18,6) |
| 16 | 12,8 (16,7) | 18,8 (27,5) | 28,0 (34,7) |
| 24 | 20,5 (25,1) | 29,2 (37,9) | 41,9 (51,3) |
| 32 | 27,6 (34,6) | 39,6 (49,7) | 56,3 (72,0) |
| 64 | 55,4 (68,4) | 79,2 (97,7) | 112 (130) |

Coût marginal d'une tile : environ **0,85 ms** en calme, **1,2 ms** en dispersé, **1,75 ms** en assaut.

### Répartition par phase, 16 tiles

| Scénario | Grilles | Perception | Déplacement |
|---|---|---|---|
| Calme | 2,2 | 6,7 | 3,8 |
| Dispersé | 2,0 | 11,0 | 5,7 |
| Assaut | 2,0 | 21,4 | 4,3 |

### Chronologie d'un assaut, une tile

| Temps de jeu | Médiane (ms) | Perception (ms) | En poursuite | Attirés par le bruit |
|---|---|---|---|---|
| 10 s | 1,29 | 0,55 | 2 670 | 15 717 |
| 60 s | 1,70 | 0,94 | 7 351 | 11 289 |
| 120 s | 2,05 | 1,34 | 12 050 | 6 766 |
| 300 s | 2,27 | 1,61 | 17 726 | 1 375 |

La densité maximale reste à 33 zombies par case de 4 m², soit 8 par m².

### Leviers de perception, dispersé, 16 tiles

| Levier | Valeur | Tick (ms) | Perception (ms) |
|---|---|---|---|
| Période de perception | à chaque tick | 62,9 | 53,9 |
| | 1 tick sur 2 | 35,2 | 27,1 |
| | **1 tick sur 5 (référence)** | **18,8** | **11,0** |
| | 1 tick sur 10 | 13,4 | 5,7 |
| | 1 tick sur 20 | 10,4 | 3,0 |
| Distance de vue | 50 m | 9,5 | 1,9 |
| | 100 m | 12,3 | 4,6 |
| | **200 m** | **18,9** | **11,0** |
| | 400 m | 34,6 | 26,4 |
| Lignes de vue tracées | 0 (ouïe seule) | 14,2 | 6,4 |
| | 1 | 18,2 | 10,4 |
| | **4** | **18,9** | **11,0** |
| | 8 | 19,5 | 11,6 |

### Taille de lot minimale

| Lot minimal | 1 tile | 16 tiles |
|---|---|---|
| 64 à 1 024 | 1,56 | 18,8 |
| 4 096 | 3,03 | 18,9 |
| 16 384 | 10,6 | 18,8 |

### Threads, dispersé, 16 tiles

| Threads | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| Tick (ms) | 208 | 105 | 53,5 | 27,9 | 18,9 |
| Accélération | ×1 | ×2,0 | ×3,9 | ×7,5 | ×11 |

La somme de contrôle est identique sur les cinq lignes.

### Mémoire et création

- **Mémoire** : environ 5,5 Mo par tile (351 Mo pour 64 tiles).
- **Création** : environ 8 ms par tile, génération de la ville comprise.

## Analyse (test 1)

**1. À ×1, environ 10 à 20 tiles parisiennes actives en même temps.** On lit la limite sur le p99 pour éviter les à-coups :

| Scénario | Tiles à ×1 (25 ms) |
|---|---|
| Calme | ≈ 20 à 24 |
| Dispersé | ≈ 14 à 16 |
| Assaut | ≈ 10 à 12 |

**2. À ×10, une seule tile, et de justesse.** Une tile calme ou dispersée tient en médiane (1,2 à 1,6 ms pour 2,5 ms) ; une tile en assaut dépasse au p99 (3,3 ms). Deux tiles ne tiennent plus. Pendant un combat en ville dense, l'accélération du temps devra donc retomber vers ×1 (A-02).

**3. Contrairement à P-00, la limite est le calcul, pas la mémoire.** Le passage à l'échelle est presque parfait jusqu'à 8 cœurs (×7,5). Les données d'une tile tiennent en cache : bitmap de 125 ko, quelques Mo de grilles. Le coût, c'est le travail par zombie : environ 10 µs de CPU par zombie et par tick en dispersé. Ajouter des tiles ne fait pas mieux que l'addition ; il n'y a pas d'économie d'échelle à attendre.

**4. La perception domine : 50 à 75 % du tick.** Son coût est proportionnel à sa fréquence (1 tick sur 5 : 11 ms ; à chaque tick : 54 ms) et à la **surface** examinée, donc au carré de la distance de vue (400 m coûte 2,4 fois plus que 200 m). Ce n'est pas le tracé des lignes de vue qui coûte le plus (≈ 40 %), c'est le **balayage des candidats** : avec une grille de 25 m, un zombie examine 289 cases pour une vue de 200 m. En assaut, chaque poursuivant parcourt les 1 000 humains du groupe, d'où la hausse progressive pendant l'attaque. C'est précisément le sujet de P-02 : la structure naïve donne ici une **borne haute**.

**5. Même au calme, une tile coûte 0,85 ms**, alors que 90 % des zombies errent sans rien percevoir. La grille, le déplacement et le balayage de perception sont payés pour tout le monde. La mise en veille des zombies sans stimulus (piste de A-03) est donc le deuxième levier, après P-02.

**6. La taille de lot de 4 096 décidée en F-02 est trop grande pour une tile seule.** 20 000 entités en lots de 4 096 ne font que 5 lots, donc 5 threads occupés sur 16 : le tick double. De 64 à 1 024, aucune différence ; avec 16 tiles, la taille n'a plus d'effet. Règle : un lot minimal d'environ 1 024, ou au plus *entités / (2 × threads)*.

**7. La reconstruction des grilles coûte 0,3 ms par tile, en série** : une tâche par tile. C'est négligeable à ×1, mais c'est 12 % du budget de ×10 pour une tile seule. On peut la paralléliser à l'intérieur d'une tile si besoin.

**8. Collisions : la capacité par case est indispensable.** Sans elle, la foule se tasse jusqu'à 1 883 zombies sur 4 m² autour des cibles. Avec une capacité et une admission progressive, la densité reste vers 6 à 8 par m², et le coût est négligeable. C'est aussi le seuil de densité dont A-05 a besoin pour l'entassement : le même mécanisme peut servir aux deux.

**9. La mémoire n'est pas un sujet** : 5,5 Mo par tile. **La création non plus** : 8 ms par tile, à étaler sur quelques frames lors de l'activation (P-04).

### Limites du test 1

- **Ville synthétique** : en 2D, sans étages, sans cours intérieures, sans zombies dans les bâtiments.
- **Humains immobiles** : ni tir, ni infection.
- **Déplacement en ligne droite** : beaucoup de zombies attirés restent bloqués contre un îlot, d'où 3 716 zombies encore « attirés » après 180 s en assaut. Un flow field (P-03) changerait la forme de la foule, donc les densités et le coût de perception.
- **Pas de frontière entre tiles** : une tile ne perçoit pas les humains de la tile voisine.
- **Perception naïve** : borne haute, en attendant P-02.

## Conclusion du test 1

- **Une tile parisienne active tient largement dans le budget de ×1** : 1,2 à 2,2 ms pour 25 ms.
- **Ordre de grandeur : 10 à 20 tiles parisiennes actives à ×1, une seule à ×10**, avec une perception naïve.
- **Le coût est du calcul pur et croît linéairement avec le nombre de tiles**. Les leviers, par ordre d'efficacité :
  1. structure de perception (P-02) ;
  2. mise en veille des zombies sans stimulus (A-03) ;
  3. fréquence de perception ;
  4. distance de vue.
- **Taille de lot** : ≈ 1 024 plutôt que 4 096.

## Test 2 — relais, veille et emprises réelles

### Ce qui change

**Relais d'alerte** (option `--relais`), selon les décisions de [ZOMBIES.md](https://github.com/fafapiti/documentation/blob/main/game-elements/zombies/ZOMBIES.md#perception) :

- Un zombie qui voit ou entend une cible publie une **alerte** : la position de la cible et l'horodatage de sa **dernière observation directe**.
- Un voisin à moins de 3 m reprend l'alerte la plus fraîche de son voisinage, sans faire sa propre perception, et la republie **avec l'horodatage d'origine**. C'est le principe des numéros de séquence des protocoles de routage (DSDV, AODV) : une information relayée ne paraît jamais plus fraîche qu'elle ne l'est. Il n'y a donc ni boucle, ni « comptage à l'infini ».
- Une alerte de plus de 3 s n'est plus reprise. Quand la source perd la cible, la vague de perte se propage en au plus 3 s plus une période de perception. C'est vérifié par le test `perte_de_trace_avec_relais`.
- **Les sources tournent d'elles-mêmes** : un zombie qui dispose d'une alerte de moins de 5 ticks ne refait pas sa perception. Sans cette règle (première version), les sources ne profitaient jamais du relais, et le gain plafonnait à 6 à 17 %. La horde est émergente : aucun objet « horde », seulement des voisinages.
- **Dernière position connue** : un zombie qui perd la cible y va encore pendant 60 s (état « perdu »), puis retourne à l'errance. La règle s'applique aussi aux zombies qui ont perçu directement.

**Mise en veille** (option `--veille`, A-03) : un zombie en errance perçoit une fois par seconde au lieu de quatre, et se déplace tous les 4 ticks d'un pas quatre fois plus long. Les règles sont les mêmes ; seule la fréquence change.

**Emprises réelles** (option `--ville reelle`) :

- [outils/extraire_tiles.py](outils/extraire_tiles.py) lit les bâtiments « En service » de la BD TOPO de Paris (édition 3.5 du 2026-09-15, voir `data/README.md`). Il rastérise chaque emprise en bitmap de 1 m, par tile de 1 km alignée sur le Lambert-93 : 105 tiles, en Python pur, en 14 s. Le rendu de la tile de l'Opéra a été vérifié à l'œil.
- Les tiles sont prises de la plus bâtie à la moins bâtie : 61 % de bâti pour la première, 43 % en moyenne sur les 64 premières.
- Commande : `python3 -I outils/extraire_tiles.py <bdtopo.gpkg> ../../data/generated/p01-tiles`.

**Déplacement par champ de distance** : dans le vrai Paris, le déplacement en ligne droite ne fonctionne plus. La foule reste bloquée derrière les immeubles. Chaque tile calcule une fois, à sa création, un **champ de distance** vers le groupe d'humains : un parcours en largeur sur le bitmap, 2 Mo par tile. Un zombie dont la cible est dans le groupe descend ce champ, comme un paquet suit sa table de routage. C'est une version minimale du flow field de P-03, valable ici parce que la cible est fixe.

**Cours fermées** : dans les tiles denses, **18 à 30 % de l'espace non bâti n'est pas accessible** depuis la rue. Ce sont des cours intérieures : la BD TOPO ne contient ni porches, ni passages sous immeuble. On place donc les agents dans le seul réseau de rues principal de la tile, la plus grande zone connexe.

Une première campagne avait placé le centre du groupe dans la case libre la plus proche du milieu de la tile. Sur la troisième tile réelle et les suivantes, c'était une cour fermée. Les 20 000 zombies s'y entassaient : 8 791 zombies sur 4 m², et 5 minutes de création. Cette campagne a été jetée.

Reproduire : `./run-variantes.sh`, qui écrit [resultats-variantes.csv](resultats-variantes.csv) et [resultats-variantes-chronologie.csv](resultats-variantes-chronologie.csv). Les assauts sont mesurés après 120 s de jeu.

### Mesures (test 2)

Temps par tick en ms, **emprises réelles**, 16 threads, lot minimal de 1 024, **médiane (p99)**. « Base » = sans relais ni veille, avec le même binaire.

#### Assaut

| Tiles | Base | Relais | Relais + veille |
|---|---|---|---|
| 1 | 2,36 (3,62) | 1,61 (1,86) | 1,66 (2,33) |
| 2 | 4,30 (5,76) | 2,80 (3,95) | 2,82 (3,48) |
| 4 | 8,18 (11,5) | 5,99 (8,04) | 6,01 (8,20) |
| 8 | 16,0 (21,5) | 11,8 (16,0) | 11,6 (15,1) |
| 16 | 33,4 (44,3) | 24,3 (29,3) | 24,3 (30,7) |
| 32 | 68,2 (81,5) | 46,8 (54,7) | 46,1 (53,5) |

#### Dispersé

| Tiles | Base | Relais | Relais + veille |
|---|---|---|---|
| 1 | 1,71 (2,39) | 1,32 (1,74) | 1,32 (1,49) |
| 2 | 2,90 (4,18) | 2,07 (2,70) | 2,08 (3,11) |
| 4 | 5,27 (7,50) | 3,69 (5,66) | 3,71 (5,64) |
| 8 | 9,95 (15,7) | 6,77 (10,2) | 6,74 (10,9) |
| 16 | 21,3 (29,3) | 14,8 (19,9) | 14,7 (23,5) |
| 32 | 44,1 (55,9) | 31,8 (41,2) | 31,3 (41,4) |

#### Calme

| Tiles | Base | Relais | Relais + veille |
|---|---|---|---|
| 1 | 1,38 (1,74) | 1,39 (1,48) | 1,04 (1,27) |
| 2 | 2,12 (2,96) | 2,13 (3,09) | 1,45 (1,79) |
| 4 | 3,60 (6,08) | 3,69 (4,95) | 2,29 (2,76) |
| 8 | 6,53 (9,58) | 6,72 (10,3) | 4,02 (5,32) |
| 16 | 14,8 (23,6) | 14,8 (20,7) | 9,82 (14,0) |
| 24 | 24,2 (28,7) | 24,8 (31,1) | 17,2 (22,1) |
| 32 | 33,0 (39,8) | 34,3 (43,1) | 24,0 (30,2) |
| 48 | 48,7 (58,9) | 52,1 (62,7) | 36,3 (44,8) |
| 64 | 65,3 (77,0) | 70,0 (84,9) | 49,9 (61,5) |

Répartition, calme, relais + veille, 64 tiles : grilles 15,4 ms (31 %), perception 22,5 ms (45 %), déplacement 11,7 ms (23 %).

#### Ville synthétique, pour comparaison

| Mesure | Base | Relais | Relais + veille |
|---|---|---|---|
| Assaut, 1 tile | 2,44 | 1,31 | 1,29 |
| Assaut, 32 tiles | 66,6 | 32,2 | 31,7 |
| Dispersé, 32 tiles | 41,5 | 30,6 | 30,6 |
| Calme, 64 tiles | 61,3 | 64,9 | 47,5 |

#### Chronologie d'un assaut, une tile réelle

| Temps de jeu | Base : tick (ms) | Base : en poursuite | Relais : tick (ms) | Relais : sources | Relais : relayés |
|---|---|---|---|---|---|
| 15 s | 1,58 | 3 629 | 1,57 | 757 | 5 324 |
| 60 s | 2,05 | 9 801 | 1,62 | 1 687 | 12 034 |
| 120 s | 2,34 | 13 127 | 1,64 | 2 617 | 12 938 |
| 240 s | 2,37 | 13 899 | 1,59 | 2 838 | 12 679 |

#### Fraîcheur de l'alerte, assaut, une tile réelle, relais

| Fraîcheur | Tick (ms) | Relayés | Perdus |
|---|---|---|---|
| 1 s | 1,62 | 12 344 | 101 |
| 3 s | 1,61 | 13 118 | 34 |
| 10 s | 1,62 | 13 177 | 9 |

#### Mémoire et création

- **Mémoire** : environ 8 Mo par tile (517 Mo pour 64 tiles). La hausse depuis le test 1 vient du champ de distance (2 Mo).
- **Création** : environ 39 ms par tile réelle, chargement et deux parcours en largeur compris.

### Analyse (test 2)

**1. Le relais tient sa promesse là où il compte : −30 à −50 % en assaut.** Sur une tile réelle en assaut, à 4 min de jeu, 2 800 sources perçoivent pour 15 500 zombies en poursuite, soit 18 %. Sans relais, la perception croît avec la foule : le tick passe de 1,6 à 2,4 ms. Avec relais, il reste plat, vers 1,6 ms. C'est l'effet « périmètre » attendu : le coût suit le nombre de sources, pas la taille de la horde. Le gain est plus fort en ville synthétique (−52 %) qu'en ville réelle (−31 %), probablement parce que les rues étroites fragmentent la foule en plus de groupes, donc en plus de sources.

**2. La veille fait le reste : −25 % au calme, rien en combat.** C'est logique : en assaut ou en dispersé, presque personne n'erre. Au calme, le relais seul coûte 3 à 7 % (publication des alertes), et la veille le rembourse largement.

**3. La ville synthétique était un bon modèle de coût.** À scénario égal, les temps de base réels et synthétiques diffèrent de moins de 10 %. En revanche, elle ne l'était pas pour le comportement : sans champ de distance, la foule réelle ne contourne pas les immeubles.

**4. Tiles tenables, emprises réelles, relais + veille** (lu sur le p99) :

| Scénario | ×1 (25 ms) | ×10 (2,5 ms) | Pour mémoire, base du test 1 à ×1 |
|---|---|---|---|
| Calme | ≈ 24 à 28 | 2 à 3 | ≈ 20 à 24 |
| Dispersé | ≈ 16 à 20 | 1 | ≈ 14 à 16 |
| Assaut | ≈ 12 à 14 | 1, de justesse | ≈ 10 à 12 |

Coût marginal d'une tile : environ **0,75 ms** au calme, **1,0 ms** en dispersé, **1,45 ms** en assaut.

**5. Le périmètre du périphérique ne tient pas : il est trois fois trop grand.** Avec la règle « tiles au contact des humains et leurs voisines », le périphérique (36 km) active **125 tiles** : 43 tiles qu'il traverse, 32 voisines intérieures et 50 voisines extérieures. Les 32 voisines intérieures n'ont pas de zombies : elles ne coûtent presque rien. Restent **environ 93 tiles avec zombies**. Même toutes au calme, en veille, elles coûtent environ 93 × 0,75 ≈ 70 ms, pour un budget de 25 ms. Il faut diviser le coût d'une tile calme par trois. Pistes, par ordre d'efficacité attendue :

- **Densité de zombies hors de Paris** : la mesure suppose 20 000 zombies par km² partout. Si la densité suit la population, la petite couronne est 2 à 3 fois moins dense que Paris (à vérifier), et les tiles extérieures coûtent d'autant moins. C'est une décision de conception, pas une optimisation.
- **Veille profonde** : au calme, la reconstruction des grilles pèse 31 % du tick, payée pour tous les zombies, même ceux qui dorment loin de tout humain. Un zombie en veille à plus de quelques centaines de mètres de tout humain n'a besoin ni de grille, ni de séparation. On peut le sortir de la grille et le déplacer une fois par seconde.
- **Structure de perception (P-02)** : 45 % du tick calme.

À l'inverse, **un assaut sur tout le périmètre à la fois** (environ 130 ms) n'est pas tenable à ×1, quelles que soient les optimisations raisonnables. Ce sera rare, mais il faut décider ce que fait le jeu dans ce cas (A-02 : ralentir sous ×1 ?).

**6. La fraîcheur ne coûte rien : c'est un choix de jeu.** Entre 1 et 10 s, le tick ne bouge pas. Seul change le délai avant que la horde ne lâche une cible perdue. 3 s donnent une vague qui porte loin sans que la horde paraisse « télépathe » longtemps.

**7. Les cours fermées sont un problème de données, pas de prototype.** 18 à 30 % de l'espace libre des tiles denses est inaccessible, parce que la BD TOPO ne contient pas les porches. Pour le jeu, il faudra percer ces accès à la génération des bâtiments (B-02, P-08). Sinon, des survivants barricadés dans une cour seraient intouchables, et des zombies y seraient enfermés.

**8. La capacité par case déborde au point de contact.** Avec le relais, la densité locale atteint 105 zombies sur 4 m² en assaut réel, contre 59 à 67 sans relais. Chaque zombie décide de son déplacement à partir du comptage du tick précédent, sans réservation. Comme dans une course entre transactions sans verrou, plusieurs entrent dans la même case au même tick. Le coût n'en souffre pas. En revanche, l'entassement de A-05 aura besoin d'une admission exacte, au moins sur les cases au contact.

**9. Mémoire et création restent négligeables** : 8 Mo et 39 ms par tile.

### Limites du test 2

- **Humains immobiles**, cible fixe : le champ de distance est calculé une seule fois. Une cible mobile demande le vrai flow field de P-03.
- **Pas de frontière entre tiles** : les alertes et la perception ne passent pas d'une tile à l'autre.
- **Pas de mur** au périphérique : le mur réduirait le nombre de zombies au contact, pas le nombre de tiles actives.
- **20 000 zombies par tile partout**, y compris sur les tiles les moins bâties.
- **Perception naïve** : borne haute, en attendant P-02.

## Conclusion

- **Une tile parisienne réelle active coûte 1,0 à 1,7 ms à ×1**, relais et veille compris.
- **Environ 12 à 28 tiles tenables à ×1 selon le scénario, 1 à 3 à ×10.**
- **Le relais d'alerte** gagne 30 à 50 % en combat, et son coût ne croît plus avec la taille de la horde. **La veille** gagne 25 % au calme. Les deux sont à garder.
- **Le périmètre du périphérique active environ 93 tiles avec zombies, trois fois le budget.** C'est le résultat principal de P-01. Il a conduit à abandonner Paris fortifié au profit de colonies installées par le joueur (A-15) : la taille du front devient un choix du joueur. Les mesures restent valables pour la reconquête d'une ville dense, qui reste le pire cas.
- **Les emprises BD TOPO** sont exploitables, mais il manque les porches.

## Décisions

Prises pendant le prototype et reportées dans les documents de conception :

- **Relais d'alerte** entre voisins, avec une perte en chaîne quand la source perd la cible ; **hordes émergentes** ([ZOMBIES.md](https://github.com/fafapiti/documentation/blob/main/game-elements/zombies/ZOMBIES.md#comportement-de-horde)).
- **Dernière position connue** suivie pendant environ une minute ([ZOMBIES.md](https://github.com/fafapiti/documentation/blob/main/game-elements/zombies/ZOMBIES.md#perception)).
- **Perception** vers 4 Hz pour un zombie actif, **veille** vers 1 Hz pour un zombie sans stimulus ([A-03](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-03--unités-dispersées-sur-tout-le-territoire)).
- **Distance de vue** plafonnée vers 100 à 200 m en ville.
- **Taille de lot minimale** d'environ 1 024 ([F-02](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation)).
- **Densité de zombies** proportionnelle à la population (A-10).
- **Veille profonde** : hors de l'index spatial, déplacement une fois par seconde, sans collision, loin de tout humain (A-01, A-03).
- **Fraîcheur de l'alerte relayée** : 3 s.
- **Dernière position connue** pour tous les zombies, y compris ceux qui ont perçu eux-mêmes.
- **Cours fermées** : passage percé à la génération des bâtiments (B-02).
- **Dépassement du budget** : le temps de jeu ralentit sous ×1 (A-02).
- **Entassement** : réservation exacte des places sur les cases au contact (A-05).
- **Base parisienne** : les 20 arrondissements, avec le périphérique comme ligne de défense. **Abandonnée ensuite** au profit de la colonisation, notamment à cause du résultat 5 du test 2 ([A-15](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-15--civils-en-métropole--colonies)).

## Questions ouvertes

- (aucune : la suite est [P-02](../README.md#p-02--perception-des-zombies))
