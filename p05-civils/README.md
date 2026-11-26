# P-05 — Civils agrégés

Prototype jetable. Problèmes : [A-15](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-15--civils-en-métropole--colonies), [A-14](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-14--territoires-sans-zombies-outre-mer-zones-reconquises), [A-17](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-17--maladies), [A-18](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-18--chaîne-sanitaire--blessures-et-usure), [B-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-02--génération-des-bâtiments), [F-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation). Fiche du prototype : [prototypes/README.md](../README.md#p-05--civils-agrégés). Points de départ : [P-00](../p00-socle-ecs/README.md) pour l'ECS, [P-04](../p04-activation/README.md) pour les zombies.

## Hypothèse testée

Le jeu doit faire vivre **1 million de colons**, et plusieurs millions en fin de partie. Chacun a :

- des jauges : faim, soif, fatigue, moral, usure ;
- une expérience et des compétences individuelles (A-18) ;
- une maladie, un logement, un travail.

Il consomme des vivres et de l'eau, et selon ses besoins des médicaments, des vêtements chauds et des couvertures. Il peut mourir (il laisse un corps) ou être zombifié.

La fiche proposait d'**agréger** les civils par bâtiment et de les **individualiser** près de la caméra ou des zombies. Il fallait d'abord **mesurer si l'agrégation est nécessaire**. P-00 donne environ 2 ms par tick pour 1 million d'entités mises à jour à chaque tick, et les jauges sont décidées à 1 Hz. Le vrai juge est **×100 hors combat** (A-02) : les colonies sont surtout en tiles sûres, et doivent tenir dans **0,25 ms par tick, économie comprise**.

Trois représentations sont comparées :

| | Représentation | Analogie |
|---|---|---|
| **A** | une entité ECS par colon, mises à jour lentes et étalées | un processus par client |
| **B** | un dossier compact par bâtiment, hors de l'ECS ; le colon devient une entité seulement quand il est individualisé | un pool de workers, et l'état des clients en base |
| **C** | un agrégat statistique par bâtiment (effectifs, moyennes) | des compteurs agrégés façon métriques |

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, SSD NVMe, Linux 7.0. Rust 1.99, `bevy_ecs` 0.20.1, compilation portable (sans `target-cpu=native`). Mesures du 2026-11-15.

## Méthode

### Bâtiments réels et colonies ([src/lieux.rs](src/lieux.rs))

[outils/extraire_batiments.py](outils/extraire_batiments.py) lit la couche `batiment` de la BD TOPO de Paris (`data/sources/bdtopo-d075-2026-09-15/`, Python pur, 8 s). Il garde les 606 117 bâtiments « En service » (Paris et petite couronne) : centroïde, surface d'emprise et attributs. Attributs vérifiés dans le fichier :

- `hauteur` (absente pour 12 % des bâtiments) ;
- `nombre_d_etages` et `nombre_de_logements` (absents pour 38 %) ;
- `usage_1` et `usage_2` : Résidentiel, Indifférencié, Annexe, Commercial et services, Industriel, Sportif, Religieux, Agricole ;
- `construction_legere`.

**Capacité** (B-02 : selon l'emprise et la hauteur ; hypothèses de prototype) :

- niveaux = nombre d'étages, sinon hauteur / 3 m ; plancher = emprise × niveaux ;
- **logement** (résidentiel, ou logements > 0) : un colon pour 35 m² ;
- **lieu de travail** (commerces et services, industrie, indifférencié sans logement, d'au moins 100 m²) : un poste pour 20 m² ;
- annexes et constructions légères : rien ;
- **chantiers** : des lieux extérieurs, un par carré habité de 500 m.

Le fichier offre 292 km² de plancher résidentiel, environ 8 millions de places ; le bloc de P-04 (55 tiles) en offre environ 2,5 millions.

**Implantation** : chaque colonie de 100 000 colons prend les logements les plus proches de son centre, pleins ; les centres sont posés en spirale autour de Paris. 60 % des colons ont un travail, à moins de 1,5 km quand c'est possible. 20 % des postes sont des chantiers extérieurs. Chaque colon a des compétences (bits) et une expérience de milieu de partie. Les colons du scénario 1 sont placés sur tout le fichier. Ceux des scénarios 2 et 3 sont placés dans le bloc de P-04, dont la carte de 1 m est chargée.

### Un colon : 32 octets, une seule fonction ([src/colon.rs](src/colon.rs))

L'état d'un colon tient dans une structure de **32 octets** :

- identifiant ;
- tick de la dernière mise à jour ;
- logement et travail ;
- cinq jauges en u16 ;
- expérience et compétences ;
- maladie et drapeaux (vêtements chauds, couverture, exposé, armé).

Le lieu où il se trouve est implicite : c'est le dossier qui le contient (B), ou un composant (A).

La fonction `vivre` fait une mise à jour sur l'intervalle **exact** écoulé depuis la précédente :

- jauges : faim, soif, fatigue, moral, usure ;
- repas et eau demandés au stock de la colonie (de l'eau non potable faute d'eau potable) ;
- froid **calculé par personne** (météo, abri chauffé ou non, vêtements chauds, couverture la nuit), et demande d'équipement ;
- maladie : risque du lieu × froid × faim et fatigue (A-17) ;
- traitement par médicaments, sinon guérison seule ou mort ;
- expérience gagnée en travaillant (A-18) ;
- cycle quotidien : prise de poste entre 6 h et 9 h pour 8 h, sommeil de 22 h à 6 h, départ et retour.

**A et B appellent la même fonction sur la même structure.** Leurs états doivent donc être identiques, ce que vérifie une somme de contrôle : c'est le même calcul, rangé autrement.

Les tirages aléatoires sont des hachages de (graine, tick, identifiant) : un seul par mise à jour, découpé en tirages de 24 bits. Les jauges sont quantifiées avec un tramage, ce qui évite qu'un petit incrément arrondi à zéro ne fasse dériver la jauge.

### Les représentations ([src/civils.rs](src/civils.rs))

Chaque lieu a une **phase**, son tick modulo la période des jauges. Les lieux sont **renumérotés par phase**, si bien que ceux d'un même tick sont contigus en mémoire. Un colon prend la phase du lieu où il se trouve.

- **A1** : une entité par colon (fiche de 32 octets, lieu, position). Chaque tick parcourt toutes les entités et ne met à jour que celles de la phase. La phase est un champ.
- **A2** : comme A1, mais les entités sont rangées dans **20 tables** par un marqueur de partition (phase modulo 20) ; chaque tick ne parcourt qu'une table. Un départ vers un lieu d'une autre partition change le marqueur ; c'est un changement rare (F-02).
- **B** : un **dossier** par lieu, un `Vec<Colon>` hors de l'ECS. Chaque tick ne parcourt que les dossiers de sa phase, découpés en lots d'au moins 1 024 colons et répartis entre les threads. Un colon qui part ou meurt sort du dossier ; l'ordre des autres est conservé.
- **C** : un **agrégat** par logement : effectifs (total, malades, équipés, actifs), moyennes des jauges et de l'expérience, effectifs par compétence. Les actifs au travail sont une fraction calculée de l'heure, et les décès un tirage sur l'effectif.

**Trajets en tile sûre** : un transfert abstrait. Le colon (B) ou son arrivée (A) entre dans une file triée par (tick d'arrivée, identifiant), et sa durée est la distance × 1,3 à 1,4 m/s. Analogie : un message en vol dans une file à échéance.

**État commun**, dans un ordre fixe en fin de tick :

- **stocks par colonie** (dépôt livré chaque jour ; la logistique est hors périmètre). Les ressources portent les noms exacts de [RESOURCES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/resources/RESOURCES.md#vivres-et-eau) : Vivres frais et en vrac, Eau potable, Eau non potable, Médicaments, Vêtements chauds, Couvertures. Le taux de service d'une fenêtre est stock / demande de la fenêtre précédente, comme un limiteur de débit ;
- **carte de risque** par tile, cases de 100 m, une tile par tick et chacune une fois par minute (A-17). Elle additionne les corps, l'eau non potable bue et la surpopulation, avec un léger flou ;
- **carte des corps** par tile, cases de 10 m (A-16). Chaque mort y ajoute un corps et un **événement daté et positionné** (le lieu du décès) pour le moral des soldats ;
- décès comptés dans le lieu, appliqués au moral et à l'usure des occupants à leur mise à jour suivante.

### Individualisation et retour à l'agrégat ([src/individus.rs](src/individus.rs))

Un lieu passe par quatre états : agrégé, individualisation en cours, individualisé, retour en cours.

- **Caméra** : les lieux à moins de 250 m sont individualisés. Ils restent individualisés tant qu'ils sont à moins de 350 m (hystérésis), et retournent à l'agrégat après le **délai**.
- **Étalement** : au plus 2 000 colons créés et 2 000 rendus par tick, créés et détruits par lots (`spawn_batch`), comme l'activation de P-04.
- Chaque lieu garde la **liste de ses entités**, comme la table des rangs d'un slot de P-04 : le retour à l'agrégat ne parcourt pas toutes les entités.
- Un colon individualisé vit avec la même fonction et la même phase. Un **trajet abstrait se fait toujours à l'état agrégé** : le colon redevient une entité à l'arrivée si le lieu est individualisé.
- **Marche réelle** sous la caméra : un colon qui part d'un lieu à moins de 350 m de la caméra marche vraiment, à 1,4 m/s. Son chemin est un **A*** sur les cases de 2 m franchissables du bloc (bitmap précalculé, 1 bit par case), entre les portes des bâtiments. Une porte est la case accessible la plus proche du centroïde. L'A* a un budget de 20 000 expansions par tick (déterministe, contrairement à un budget en temps). Au-delà de 60 000 expansions, le trajet redevient abstrait.
- **Variante « colon qui sort travailler »** : les chantiers extérieurs sont toujours individualisés.

Sans marche réelle, une simulation avec caméra doit être **identique** à une simulation sans caméra.

### Assaut d'une colonie isolée ([src/assaut.rs](src/assaut.rs))

La simulation des zombies de P-04 (copiée dans [src/](src)) tourne sur le bloc réel de 55 tiles. Une colonie de **30 000 colons dans 574 bâtiments** occupe la tile (654, 6861). Cette tile est vidée de ses zombies ; ses huit voisines sont pleines (densité 1 : environ 12 000 à 17 000 zombies chacune ; 0,25 pour la variante « banlieue »). L'assaut commence à 23 h, quand tout le monde est chez soi, et dure 30 min de jeu.

- **Perception** : un bâtiment occupé est **un point à sa porte**, dans l'arbre k-d des humains de P-02. Variante : un point par occupant, à moins de 1 m de la porte.
- **Tirs par les fenêtres** : le tir de P-04 (une chance sur deux par seconde, à 25 m), une fois par occupant armé non exposé. Une part des colons est armée (0, 5 ou 20 %).
- **Entrée** : un zombie à moins de 2 m de la porte d'un bâtiment occupé y entre (état `DEDANS`). Il sort de la grille et ne bouge plus, mais reste une entité.
- **Auto-résolution** chaque seconde (B-02) : chaque occupant armé abat un zombie avec une chance de 0,3 ; chaque zombie expose un occupant avec une chance de 0,25.
- **Zombification** 10 s après l'exposition : le colon quitte le dossier, et une **entité zombie** naît dans le bâtiment.
- Un bâtiment sans humain libère aussitôt ses zombies, qui reprennent l'errance. Il n'est plus perçu.
- **Aucune règle ne déclenche la chute** de la colonie.

### Scénarios ([src/main.rs](src/main.rs))

| Scénario | Mode | Mesure |
|---|---|---|
| 1. Colonie calme | `calme` | coût par tick de 100 000 à 3 millions de colons (A1, A2, B, C ; jauges toutes les 1 s, 10 s, 30 s, 60 s) ; fenêtre de 2 min à 6 h 30 (prises de poste), 3 h, 12 h et 18 h |
| 2. Survol | `survol` | 1 million de colons dans le bloc ; caméra en aller-retour sur 3 km au centre, à 10, 50 et 200 m/s, pendant 10 min à partir de 7 h ; délai de 0 à 60 s |
| 3. Assaut | `assaut` | colonie isolée de 30 000 colons, 30 min ; part armée, perception, densité |
| 4. Hiver | `hiver` | 100 000 colons, 7 jours, −5 °C ±4, sans chauffage, avec ou sans vêtements chauds, couvertures et médicaments |
| Sauvegarde | `sauvegarde` | 1 million de colons après 1 h de jeu : taille par colon dans la sauvegarde d'une tile |

### Déterminisme et contrôles

- **Tests** (`cargo test --release`, 17 tests dont 7 de bout en bout sur une ville et un bloc synthétiques, [src/tests.rs](src/tests.rs)) :
  - `conservation_a_chaque_tick` : pour B, A2 et C, vivants + morts + zombifiés = initial **à chaque tick**, avec des maladies et des morts forcées. Aucun doublon d'identifiant (A, B). Un corps et un événement par mort ;
  - `a1_a2_b_identiques` : même somme de contrôle ;
  - `independant_du_decoupage` : lots de 1 à 100 000 ;
  - `individualisation_aller_retour` : toute la ville individualisée puis rendue à l'agrégat, sans doublon, avec conservation à chaque tick. À la fin, **chaque colon est identique, champ par champ**, à celui d'une simulation sans caméra : identité, expérience, compétences et jauges ;
  - `sauvegarde_aller_retour` ;
  - `assaut_conservation` : conservation des humains et des zombies à chaque tick, et **une entité zombie par zombifié**.
- **Campagne** : la colonne `somme` des CSV est identique de 1 à 16 threads et pour des lots de 64 à 100 000. Elle est aussi identique entre A1, A2 et B à chaque effectif et à chaque période, entre les heures de mesure de A2 et de B, et entre un survol sans marche réelle et une simulation sans caméra. Dans l'assaut, elle ne change pas de 1 à 16 threads.
- `clear_trackers` à chaque tick ; créations et destructions par lots ; préchauffage des tables de P-04 (250 000 zombies).

Reproduire : `./run.sh` (environ 50 min ; `ESSAI=1 ./run.sh` valide toutes les commandes en 3 min). Le script reconstruit le binaire release. Données : `python3 -I outils/extraire_batiments.py <gpkg> ../../data/generated/p05-batiments`.

### Paramètres de prototype

Ce sont des **hypothèses de mesure**, pas des statistiques de jeu :

- 35 m² par colon, 20 m² par poste ; 60 % d'actifs, dont 20 % sur des chantiers ;
- faim de 0 à 1 en 16 h et repas de 0,25 ; soif en 8 h et prise de 0,2 ;
- maladie de base de 0,004 par jour à risque 1, ×(1 + 6 × froid) ;
- guérison de 0,4 par jour avec médicaments et de 0,08 sans ; mort de 0,15 par jour sans médicaments ;
- rayon de caméra de 250 m et retour au-delà de 350 m ;
- assaut : chance de tuer de 0,3 et d'exposer de 0,25 par seconde, entrée à 2 m.

## Budgets de référence

Les mêmes que pour P-00 à P-04 : 25 ms par tick à ×1, 2,5 ms à ×10, **0,25 ms à ×100**. Le coût d'un tick ne dépend pas de l'accélération : à ×100 on joue 100 fois plus de ticks par seconde réelle, chacun avec un budget 100 fois plus petit.

## Mesures

### 1. Colonie calme ([resultats-calme.csv](resultats-calme.csv))

Tick médian (p99) en ms, 16 threads, à 6 h 30 (prises de poste). Le tick comprend tout le travail des civils : arrivées, mises à jour, départs, décès, économie, carte de risque.

**Jauges à 1 Hz** (période de 20 ticks) :

| Colons | A1 | A2 | B | C |
|---|---|---|---|---|
| 100 000 | 0,27 (0,46) | 0,11 (0,17) | 0,074 (0,11) | 0,019 (0,027) |
| 1 million | 2,06 (2,57) | 0,39 (0,51) | **0,25 (0,31)** | 0,064 (0,088) |
| 3 millions | 5,98 (7,17) | 0,98 (1,31) | 0,63 (0,77) | 0,29 (0,32) |

**Jauges à 0,1 Hz** (période de 200 ticks) :

| Colons | A1 | A2 | B | C |
|---|---|---|---|---|
| 100 000 | 0,17 (0,37) | 0,067 (0,093) | 0,027 (0,047) | 0,013 (0,020) |
| 1 million | 0,96 (1,24) | 0,21 (0,28) | **0,082 (0,12)** | 0,017 (0,028) |
| 3 millions | 2,77 (3,75) | 0,51 (0,74) | **0,15 (0,21)** | 0,040 (0,055) |

Pour 1 million de colons, le même tick au regard des trois budgets :

| | 1 Hz | Part du budget, ×1 / ×10 / ×100 | 0,1 Hz | Part du budget, ×1 / ×10 / ×100 |
|---|---|---|---|---|
| A1 | 2,06 ms | 8 % / 82 % / **820 %** | 0,96 ms | 4 % / 38 % / **380 %** |
| A2 | 0,39 ms | 2 % / 16 % / **160 %** | 0,21 ms | 1 % / 8 % / **84 %** |
| B | 0,25 ms | 1 % / 10 % / **100 %** | 0,082 ms | 0,3 % / 3 % / **33 %** |
| C | 0,064 ms | 0,3 % / 3 % / 26 % | 0,017 ms | 0,1 % / 0,7 % / 7 % |

**Périodes plus longues** (B / A2, médiane en ms) :

| Période | 1 million | 3 millions |
|---|---|---|
| 10 s | 0,082 / 0,21 | 0,15 / 0,51 |
| 30 s | 0,057 / 0,16 | 0,093 / 0,37 |
| 60 s | 0,042 / 0,15 | 0,081 / 0,33 |

**Heure de la journée** (B / A2, 0,1 Hz, 1 million) : 3 h 0,074 / 0,21 ; 6 h 30 0,082 / 0,21 ; 12 h 0,075 / 0,18 ; 18 h 0,081 / 0,21 ms. Les prises de poste créent 2,8 départs par tick en moyenne ; ils coûtent moins de 0,01 ms.

**Threads** (B, 1 million, médiane en ms ; somme identique sur chaque ligne) :

| Threads | 1 | 4 | 8 | 16 |
|---|---|---|---|---|
| 1 Hz | 1,40 | 0,39 | 0,27 | 0,25 |
| 0,1 Hz | 0,157 | 0,076 | 0,079 | 0,075 |

**Lots** (B, 0,1 Hz) : 64 et 1 024 donnent 0,08 ms ; 100 000 (un seul lot, donc un seul thread) donne 0,16 ms. Trois répétitions de B et de A2 à 0,1 Hz : 0,076 à 0,084 ms et 0,209 à 0,211 ms.

**Mémoire résidente par colon** : A1 146 à 156 octets, A2 150 à 163, **B 45 à 50** (32 octets de fiche, plus la capacité de réserve des tableaux), C 2 à 10 (un agrégat par logement). Pour 3 millions de colons : environ 490 Mo en A2, 150 Mo en B. Installation de 3 millions de colons (hors lecture des bâtiments) : 0,1 s en B, 0,35 s en A1, 0,5 s en A2.

**Micro-banc** ([banc-vivre.txt](banc-vivre.txt), un thread) : **25 ns** par mise à jour, de 4 500 à 1 million de colons. Le coût est du calcul, pas de la mémoire.

### 2. Survol ([resultats-survol.csv](resultats-survol.csv))

1 million de colons dans le bloc, jauges à 1 Hz, 10 min de jeu à partir de 7 h, budget de ×1. Toutes les portes sont trouvées. Conservation vérifiée, aucun doublon.

| Caméra | Délai | Médiane | p99 | Max | Colons individualisés / min | Rendus / min | Rebascules | Entités au plus |
|---|---|---|---|---|---|---|---|---|
| 10 m/s | 0 s | 0,29 | 1,59 | 12,4 | 9 200 | 9 270 | 1 656 | 18 000 |
| 10 m/s | 60 s | 0,30 | 1,60 | 12,5 | 9 200 | 8 520 | 1 651 | 37 200 |
| 50 m/s | 0 s | 0,30 | 1,52 | 12,5 | 46 200 | 46 200 | 14 820 | 18 100 |
| 50 m/s | 10 s | 0,31 | 1,50 | 12,4 | 46 100 | 45 800 | 14 780 | 34 800 |
| 50 m/s | 30 s | 0,32 | 1,55 | 12,4 | 36 700 | 33 200 | 11 380 | 48 900 |
| 50 m/s | 60 s | 0,32 | 1,81 | 12,4 | 16 900 | 12 600 | 4 700 | 49 500 |
| 200 m/s | 0 s | 0,33 | 1,67 | 9,8 | 184 600 | 184 700 | 64 180 | 18 200 |
| 200 m/s | 10 s | 0,34 | 1,84 | 9,9 | 117 200 | 112 700 | 40 410 | 49 200 |
| 200 m/s | 30 s | 0,32 | 2,12 | 9,7 | 4 300 | 0 | 0 | 49 500 |

Sans caméra, la médiane est de 0,26 ms et le p99 de 0,32 ms. Une **rebascule** est un lieu individualisé de nouveau après être retourné à l'agrégat : c'est du travail gaspillé. À 200 m/s, l'aller-retour de 3 km dure 30 s : avec un délai de 30 s, plus rien ne retourne à l'agrégat, et tout le couloir (49 500 colons) reste individualisé.

- **Individualisation d'un bâtiment** : **1 tick** (2 ticks au plus avec des lots de 500), car les bâtiments font quelques dizaines de colons. Elle coûte **80 à 160 ns par colon** ; le retour, 180 à 290 ns par colon. À 200 m/s sans délai, le poste coûte 0,04 ms par tick (caméra, individualisation et retour).
- **Marche réelle** : 340 à 410 chemins en 10 min, au plus 250 à 320 marcheurs. **1,4 à 1,6 ms par chemin** en moyenne, **9,4 ms au plus** ; 7 à 9 % de chemins introuvables en 60 000 expansions, qui redeviennent des trajets abstraits. Un colon attend au plus 0,5 s devant sa porte faute de budget. La marche ajoute 0,05 ms par tick en moyenne. **Le max du tick (10 à 12 ms) vient d'un seul chemin long** : le budget par tick laisse toujours passer au moins un chemin.
- **Budgets de ×10** (lots de 500, 5 000 expansions par tick) : p99 1,57 ms, mais le max reste à 9,6 ms. Un chemin ne se découpe pas.
- **Équivalence** : sans marche réelle, un survol à 50 m/s (36 700 colons individualisés par minute) donne **la même somme de contrôle** qu'une simulation sans caméra (`a696ae183191d1dc`).
- **Chantiers toujours individualisés** (10 h, sans caméra) : **123 700 entités**, tick de 0,32 ms au lieu de 0,23 (+38 %), même somme que sans individualisation.

### 3. Assaut

Fichiers : [resultats-assaut.csv](resultats-assaut.csv), chronologies `chrono-assaut-*.csv`.

30 000 colons, 574 bâtiments, 9 tiles actives, 30 min de jeu, 16 threads. Médianes en ms par tick.

| Variante | Tick (p99) | Max | Humains (arbre) | Perception | Entrées | Résolution | Zombifications | Civils |
|---|---|---|---|---|---|---|---|---|
| 0 % d'armés | 3,67 (4,66) | 8,6 | 0,16 | 0,74 | 0,022 | 0,0005 | 0,031 | 0,038 |
| **5 % d'armés** | **2,23 (2,59)** | 6,1 | **0,31** | 0,32 | 0,037 | 0,0001 | 0,054 | 0,053 |
| 20 % d'armés | 2,60 (3,03) | 8,8 | 0,68 | 0,32 | 0,039 | 0,0001 | 0,052 | 0,054 |
| 5 %, **un point par occupant** | 5,55 (6,63) | 9,6 | **3,13** | 0,34 | 0,036 | 0,0001 | 0,10 | 0,053 |
| 5 %, banlieue (densité 0,25) | 1,56 (1,74) | 5,3 | 0,31 | 0,14 | 0,037 | 0,0001 | 0,044 | 0,052 |

Avec 1 point par bâtiment, la perception reçoit 574 points ; avec un point par occupant, 30 000. La phase « humains » comprend les tirs par les fenêtres : elle croît avec le nombre de tireurs.

| Variante | Entrées | Abattus (dont à l'intérieur) | Zombifiés | Bâtiments tombés | Humains à la fin | Zombies à la fin |
|---|---|---|---|---|---|---|
| 0 % d'armés | 223 267 | 0 | **29 565** | **571 sur 574** | 435 | 132 895 |
| 5 % | 2 402 | 2 779 (90) | 957 | 29 | 29 043 | 101 508 |
| 20 % | 0 | 1 786 | 0 | 0 | 30 000 | 101 544 |
| 5 %, un point par occupant | 15 435 | 4 798 (143) | 2 640 | 64 | 27 360 | 101 172 |
| 5 %, banlieue | 759 | 882 (17) | 470 | 16 | 29 530 | 25 421 |

Chronologie à 0 % d'armés ([chrono-assaut-armes0.csv](chrono-assaut-armes0.csv)) : 545 bâtiments tiennent à 3 min, 405 à 9 min, 172 à 18 min, 12 à 27 min. Le nombre de zombies passe de 103 000 à 133 000. À 5 % ([chrono-assaut-armes0.05.csv](chrono-assaut-armes0.05.csv)), les pertes cessent après 12 min.

**Conservation** des humains (vivants + morts + zombifiés) et des zombies (P-04, plus les zombifiés) vérifiée à chaque seconde de jeu dans chaque variante. **Chaque zombifié est devenu une entité zombie** (29 565 sur 29 565 à 0 %). Déterminisme : même somme (`f93335d57b300030`) à 16, 8, 4 et 1 thread sur 5 min (16 bâtiments tombés, 593 zombifiés). Le tick passe de 2,25 ms à 16 threads à 10,1 ms à un thread.

### 4. Hiver

Fichiers : [resultats-hiver.csv](resultats-hiver.csv), chronologies `chrono-hiver-*.csv`.

100 000 colons (une colonie), jauges à 0,1 Hz, 7 jours de jeu, 16 µs par tick (une semaine en 3 min 15 s).

| Variante | Malades au jour 7 | Morts (corps) | Actifs disponibles (début → jour 7) | Heures travaillées (jour 1 → jour 7) | Risque max au jour 7 |
|---|---|---|---|---|---|
| Référence : 2 °C, chauffage, stocks | 852 | **0** | 59 766 → 59 220 (−0,9 %) | 477 000 → 474 000 | 0,50 |
| −5 °C sans chauffage, vêtements et couvertures, **sans médicaments** | 3 166 | **1 730** | → 56 683 (−5,2 %) | 477 000 → 456 000 | 1,91 |
| −5 °C sans chauffage, **sans rien** (B) | 11 845 | **5 246** | → 49 334 (**−17,5 %**) | 475 000 → 406 000 (−15 %) | 4,86 |
| Même hiver, **agrégat C** | 21 402 | **8 534** | → 45 677 (−23,6 %) | 475 000 → 381 000 | 7,47 |

Les morts croissent chaque jour (96, 296, 507, 672, 896, 1 228, 1 551 sans rien) : les corps ne sont pas évacués, ils montent la carte de risque (0,50 → 4,9 au plus), qui rend malade. La boucle « corps → maladie » de A-16 et A-17 apparaît sans règle dédiée.

### Sauvegarde ([resultats-sauvegarde.csv](resultats-sauvegarde.csv))

1 million de colons après 1 h de jeu, 72 tiles, 10 900 colons par tile en médiane.

| | Par colon |
|---|---|
| En mémoire (B) | 32 octets |
| Sauvegarde brute (varints, écarts) | 19,4 octets |
| **Sauvegarde zstd** | **15,9 octets** (p90 18,2) |

Une tile médiane fait **173 ko**, la plus peuplée 854 ko. Encodage 1,7 ms et décodage 0,6 ms par tile. Aller-retour identique, avec somme de contrôle. À comparer avec les **4 octets par zombie** de P-04 : les cinq jauges en u16, presque aléatoires, font la moitié du poids.

## Analyse

**1. L'agrégation statistique n'est pas nécessaire ; un rangement compact l'est.** Les trois représentations individuelles A1, A2 et B font **exactement le même calcul** (même somme de contrôle partout), et leurs coûts diffèrent d'un facteur 12. À 1 million de colons et 0,1 Hz :

- **A1** : le parcours de toutes les entités domine. Lire un million de phases pour en traiter 4 500 coûte 0,96 ms, quatre fois le budget de ×100. C'est un `SELECT *` suivi d'un `WHERE` côté client.
- **A2** : ne parcourt qu'une table sur 20, à 0,21 ms (84 % du budget). Il paie encore le parcours d'une table entière (50 000 entités pour 4 500 mises à jour), 20 systèmes par tick et des fiches dispersées entre composants : 3,3 fois la mémoire de B.
- **B** : à 0,082 ms (33 % du budget), il ne touche que les dossiers de sa phase, contigus, 32 octets par colon. C'est un index sur la colonne `phase`, avec les données dans l'index.
- **C** : 4 à 5 fois moins cher que B, mais B tient déjà largement le budget.

Analogie : A est un processus par client, qui coûte même endormi ; B est un pool de workers qui ne réveille que les dossiers à traiter.

**2. La fréquence des jauges est le premier levier, avant la représentation.** À 1 Hz, B consomme à lui seul tout le budget de ×100 (0,25 ms), sans rien laisser à la production, à la logistique et aux navires. À 0,1 Hz, B tombe à 0,08 ms ; à 30 s, 0,06 ms. La précision n'en souffre pas : chaque mise à jour intègre l'intervalle **exact** écoulé, et la faim ou la soif changent sur des heures. Le coût ne décroît pas en proportion au-delà de 10 s : il reste un coût fixe d'environ 0,04 ms (exécuteur, lots, arrivées, économie).

**3. B passe à l'échelle jusqu'à 3 millions.** À 0,1 Hz, 3 millions de colons coûtent 0,15 ms (60 % du budget de ×100) ; à 30 s, 0,09 ms. La limite n'est pas la bande passante mémoire, contrairement à P-00. On ne touche que 1/200 des colons par tick (50 ko à 1 million), et le banc montre 25 ns par colon, chaud ou froid. **4 threads suffisent** à 0,1 Hz : au-delà, le gain est nul. Les autres cœurs restent libres pour le reste de l'économie.

**4. C casse ce qu'A-18 exige, et donne d'autres résultats.**

- **Identité** : un agrégat sait que « 3 soudeurs » vivent dans l'immeuble, pas lesquels. Un colon individualisé depuis C serait **inventé** : son expérience et ses compétences seraient tirées au sort, et rendues à l'agrégat, elles seraient fondues dans une moyenne. L'expérience ne peut donc pas suivre la personne (A-18 : colon devenu ouvrier, puis milicien, puis soldat).
- **Qui meurt, qui est malade, qui est armé** : C ne peut que tirer au sort dans les effectifs. Les identifiants des morts de C sont synthétiques.
- **Le calcul lui-même diverge** : en hiver, C compte **63 % de morts de plus** que B (8 534 contre 5 246) avec les mêmes paramètres. Le froid, la faim et la maladie ne sont pas linéaires : appliqués à la moyenne, ils ne donnent pas la moyenne des individus (inégalité de Jensen). La boucle corps → risque amplifie l'écart. C serait un autre modèle, à calibrer contre B, pas un raccourci du même.
- Le seul avantage de C, un coût 4 à 5 fois plus faible, ne sert à rien : B tient déjà le budget.

**5. Un bâtiment est un contenant : il n'y a rien à individualiser à l'intérieur.** Pendant le survol, une caméra qui individualise les bâtiments dans un rayon de 250 m fait jusqu'à **49 500 entités**, pour **250 à 320 marcheurs** visibles. Les autres sont dans des bâtiments dont l'intérieur n'est pas simulé (B-02) : en entité, ils font exactement ce qu'ils faisaient en dossier, ce que prouve l'équivalence des sommes. Le coût reste faible (0,3 ms à ×1, 80 à 290 ns par colon), mais c'est du travail sans effet visible. Il en va de même pour l'assaut : **les occupants restent dans leur dossier, et la tile active n'a besoin d'aucune individualisation**. Les combats intérieurs sont auto-résolus sur les dossiers, et seul le zombifié devient une entité. Le seul colon qui ait besoin d'une entité est celui qui est **dehors** : en marche sous la caméra, ou exposé dans la rue d'une tile active (ce dernier cas n'est pas mesuré).

**6. Le délai ne sert qu'à éviter le gaspillage, comme dans P-04.** À 50 m/s, sans délai, 14 800 lieux sont rebasculés en 10 min ; avec 60 s, 4 700. Un délai long garde plus d'entités vivantes (jusqu'à 49 500), sans coût visible. Aucune variante ne dépasse 2,1 ms au p99 à ×1. L'hystérésis en distance (250 m pour entrer, 350 m pour sortir) évite les bascules en bord de cercle.

**7. La marche réelle est le seul poste lourd, à cause du calcul des chemins.** Un A* sur les cases de 2 m coûte 1,4 ms en moyenne et 9,4 ms au plus. Il est déterministe grâce à son budget d'expansions, mais **indivisible** : un chemin long fait un tick de 10 à 12 ms. C'est sans gravité à ×1 (25 ms), mais c'est 4 fois le budget de ×10. Et 7 à 9 % des portes ne communiquent pas en 60 000 expansions (cours fermées, passages de moins de 2 m que B-02 percera). Deux pistes : un A* reprenable d'un tick à l'autre, ou un champ de distance par destination, partagé par tous les colons qui y vont (comme les flow fields des zombies). La première version de l'A*, qui relisait la carte de 1 m quatre fois par case, coûtait 18 ms par chemin ; un bitmap des cases de 2 m l'a divisée par 12.

**8. Un point pondéré par bâtiment suffit à la perception, et coûte 10 fois moins qu'un point par occupant.** Avec 574 points, la phase « humains » (arbre k-d de 9 tiles et tirs) coûte 0,31 ms ; avec 30 000, 3,13 ms, et le tick passe de 2,2 à 5,5 ms. P-02 l'avait prévu : 46 µs par millier d'humains, reconstruit à chaque tick. Le comportement change aussi : 30 000 points autour des portes attirent davantage (2 640 zombifiés contre 957). Ce sont des points qui ne bougent pas et ne se distinguent pas : un seul suffit, porteur du nombre de tireurs.

**9. L'auto-résolution ne coûte rien ; la chute d'une colonie émerge.** La résolution coûte 0,0001 à 0,0005 ms par tick, les entrées 0,02 à 0,04 ms, les zombifications 0,03 à 0,10 ms. Le coût de l'assaut est celui des zombies de P-04. La chute est une **réaction en chaîne** sans règle dédiée :

- un zombie entre et expose ;
- 10 s plus tard, les exposés sont des zombies, à l'intérieur ;
- le bâtiment se vide, et 50 zombies en sortent vers le voisin.

Sans arme, 571 bâtiments sur 574 tombent en 30 min ; avec 5 % de miliciens, la colonie perd 29 bâtiments puis tient ; avec 20 %, aucun zombie n'atteint une porte. Analogie : une infection latérale dans un réseau plat, que la segmentation (les armes à chaque porte) arrête.

**10. L'hiver ne coûte rien en calcul, et c'est la logistique qui décide.** Sans vêtements chauds, couvertures ni médicaments, 5,2 % des colons meurent en une semaine, et la main-d'œuvre disponible baisse de 17,5 %. Avec des vêtements chauds et des couvertures, mais sans médicaments, les morts sont divisées par 3 ; avec tout, il n'y en a aucune. Les corps non évacués font monter le risque jour après jour : la boucle de A-16 et A-17 fonctionne telle quelle.

**11. Un colon pèse 16 octets dans la sauvegarde.** C'est 4 fois un zombie, parce qu'un colon a un état et qu'un zombie n'en a presque pas. 1 million de colons font 16 Mo. Quantifier les jauges sur un octet dans la sauvegarde gagnerait environ 5 octets par colon, mais casserait l'identité stricte de l'aller-retour.

**12. Deux pièges de mesure, à retenir pour tout le jeu.**

- **`floor` coûte un appel de bibliothèque** dans un binaire portable (x86-64 de base, sans SSE4.1). Avec cinq `floor` et un `rem_euclid` par colon, une mise à jour coûtait 41 ns ; avec des troncatures sur des valeurs positives, 25 ns. Le hachage, lui, ne pesait presque rien (41 → 39 ns).
- **Un taux de service initial** remis à 1 par un bilan sans demande servait un stock vide pendant toute la première fenêtre : chaque colon recevait des vêtements inexistants. La première campagne (archivée dans `data/generated/p05-premiere-campagne/`) avait ce défaut, et une unité de médicaments fausse. Ses coûts sont identiques ; ses résultats d'hiver ne le sont pas.

### Limites

- **Pas de logistique** : un dépôt par colonie, livré chaque jour ; pas de production. Le coût d'une économie complète n'est pas mesuré : il reste environ 0,17 ms du budget de ×100 pour elle.
- **Pas de soldats** : les événements de mort sont produits (position, tick), pas consommés.
- **Assaut de nuit** : personne n'est dehors. Le contact d'un colon exposé dans la rue n'est pas mesuré, ni l'individualisation des colons dehors à l'activation d'une tile. Le **bruit des tirs** n'attire pas les zombies : le champ « bruits » de P-03 avait été retiré dans P-04.
- **Combats intérieurs** à paramètres fixes : ni armes, ni munitions consommées.
- **C est sommaire** : le travail et la consommation d'eau y sont des fractions moyennes. Son écart avec B vient en partie de ce modèle, pas seulement de l'agrégation.
- **Colonies sans enfants, sans âge, sans famille** ; capacités de 35 et 20 m² par personne, à confirmer.
- **Marche réelle** dans le bloc de P-04 seulement (la carte de 1 m n'existe que là). Les portes sont approchées par la case accessible la plus proche du centroïde.
- **L'exécuteur de Bevy** est compté dans le tick des civils (un ou vingt systèmes) : dans le jeu, il sera partagé.
- **Bruit de mesure** : des ticks isolés de 1,5 à 3,4 ms au-dessus du p99 au calme, sur une seule ligne et non reproduits (répétitions de 0,15 à 0,44 ms au max).

## Conclusion

- **Représentation : B, des dossiers compacts par bâtiment.** Elle fait le même calcul que des entités ECS, colon par colon, pour 2,5 à 12 fois moins cher et 3 fois moins de mémoire : 1 million de colons en **0,08 ms par tick** à 0,1 Hz (33 % du budget de ×100), 3 millions en 0,15 ms. L'agrégat statistique (C) n'est pas nécessaire, et il casse A-18.
- **Jauges des civils à 0,1 Hz** (toutes les 10 s) en tile sûre. À 1 Hz, les civils prennent tout le budget de ×100 à 1 million.
- **Rien à individualiser dans un bâtiment** : seuls les colons dehors deviennent des entités. Individualiser, rendre et marcher est exact et peu coûteux (80 à 290 ns par colon, 1 tick par bâtiment).
- **Un point pondéré par bâtiment occupé** pour la perception : 10 fois moins cher qu'un point par occupant.
- **La chute d'une colonie émerge** de l'auto-résolution et de la zombification, avec une conservation exacte et une entité zombie par zombifié ; son coût est celui des zombies.
- **Sauvegarde** : 16 octets par colon.
- Le seul poste à revoir est le **chemin à pied sous la caméra** : jusqu'à 10 ms d'un tenant.

## Décisions

Prises par l'utilisateur sur les recommandations de ce prototype, et reportées dans [OPEN-PROBLEMS.md](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-15--civils-en-métropole--colonies) (A-01, A-07, A-13, A-14, A-15, A-17, B-02, F-02), [SYSTEMES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/systemes/SYSTEMES.md#colonisation), [ZOMBIES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/zombies/ZOMBIES.md#perception), [RESOURCES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/resources/RESOURCES.md#main-dœuvre) et la fiche [Civils](https://github.com/labatailledefrance/documentation/blob/main/game-elements/units/soutien/civils.md).

- **Représentation** : dossiers compacts par bâtiment (B), hors de l'ECS. L'agrégat statistique (C) est abandonné.
- **Individualisation** : seulement les colons dehors (marche sous la caméra, rue d'une tile active) ; jamais les occupants d'un bâtiment, même quand la tile devient active ou qu'un zombie arrive à portée. Un chantier extérieur en tile sûre est un lieu agrégé comme un bâtiment.
- **Retour au dossier** : 30 s après la fin de la raison d'être individuel, avec 100 m d'hystérésis sur le rayon de la caméra.
- **Jauges des civils** : toutes les 10 s en tile sûre, sur l'intervalle exact (30 s au-delà de 2 millions de colons) ; même période pour un colon individualisé. Outre-mer, 30 à 60 s dans les tiles sans colonie ni caméra.
- **Perception** : un bâtiment occupé est un point à sa porte, porteur du nombre de tireurs. Les tirs par les fenêtres alimentent le champ « bruits ».
- **Combat intérieur** : chances selon l'arme, la munition et le nombre (valeurs à définir) ; munitions consommées dans le dépôt de la mairie de la commune ; sans munitions, les occupants ne se battent plus.
- **Porte** : une seule par bâtiment, générée avec lui (P-08) ; case accessible la plus proche dans les prototypes en attendant.
- **Marche** : A\* reprenable d'un tick à l'autre ; trajet abstrait quand aucun chemin à pied n'existe, en tile sûre seulement (en tile active, le colon reste dans son bâtiment).
- **Sauvegarde** : sans perte, environ 16 octets par colon.
- **Capacité** : 35 m² de plancher par colon logé, 20 m² par poste de travail (valeurs de départ).
- **Population** : pas d'âge modélisé ; le temps ne passe pas pour elle (ni vieillissement, ni naissances).
- **Bruit** : un tir par une fenêtre porte aussi loin qu'un tir dehors.
- **Dépôt** : pas d'objet « colonie » ; la mairie de chaque commune est le dépôt de toutes les ressources de ses colons, où chaque bâtiment puise sans transport simulé. Mairie détruite : à réparer, ou à reconstruire à l'identique si elle s'est effondrée ; mairie tombée aux zombies : stock laissé sur place, inaccessible tant qu'elle n'est pas reprise. Capacité illimitée ; soldats et ouvriers logés dans la commune y puisent aussi, munitions comprises (une unité en opération passe par sa chaîne logistique). La mairie n'est que le dépôt par défaut : pour sa ressource, un bâtiment puise au dépôt de munitions ou de carburant construit le plus proche, dans sa commune ou une voisine ; le joueur transporte lui-même le stock d'un dépôt à l'autre. Commune voisine : une commune qui touche celle du bâtiment ; un dépôt vide ou occupé par des zombies est sauté ; les autres ressources des colons restent à la mairie. Une mairie par commune (la principale) ; à Paris, Lyon et Marseille, chaque arrondissement compte comme une commune ; sans mairie dans les données, le plus grand bâtiment de la commune. Le prototype, lui, a un dépôt par colonie : à l'échelle mesurée, c'est le même calcul (une demande par lieu, un stock par dépôt).

## Questions ouvertes

Aucune pour l'instant : les questions de ce prototype et celles qu'ont soulevées ses décisions sont toutes tranchées.
