# P-07 — Chaîne de données IGN sur la Manche

Prototype jetable. Problèmes : [B-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-01--chaîne-de-traitement-des-données-ign), [A-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-01--nombre-de-zombies--simulation-par-tiles), [A-10](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-10--population-civile), [A-13](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-13--sauvegardes), [A-19](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-19--suivi-des-zombies-par-commune), [B-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-02--génération-des-bâtiments), [B-03](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-03--charge-limite-des-ponts), [B-04](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-04--végétation), [B-06](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-06--licences-des-données), [C-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-01--précision-numérique), [C-04](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-04--streaming-sans-écran-de-chargement). Fiche du prototype : [prototypes/README.md](../README.md#p-07--chaîne-de-données-ign-sur-la-manche). Premier prototype sur des données hors de Paris, et premier à produire les **tiles du jeu** elles-mêmes.

## Hypothèse testée

Le monde du jeu est construit à partir des données brutes de l'IGN (BD TOPO, RGE ALTI) et de l'INSEE (carreaux de population). Il faut une **chaîne** qui les transforme en tiles de 1 km sur la grille du Lambert-93 (C-01), pour toute la France, sur une machine de joueur. Elle doit établir :

1. le **contenu** d'une tile, couche par couche, avec sa résolution, son format et sa taille ;
2. ce qui se **précalcule** et ce qui se **recalcule au chargement** ;
3. la réponse à **B-01** : traitement **avant distribution** (gros téléchargement) ou **à l'installation** (long calcul local) ;
4. le **déterminisme** : mêmes entrées, mêmes octets, quel que soit le nombre de threads, condition de A-13 (monde de base régénéré, sauvegarde des seules différences) ;
5. les **raccords** : relief continu, et chaque bâtiment ou route dans une seule tile, sans perte ni doublon.

Analogie : la chaîne est un pipeline de build. Les sources (BD TOPO, RGE ALTI, INSEE) sont le code ; les tiles sont l'artefact. La question de B-01 est celle de toute distribution logicielle : livrer des binaires (avant distribution) ou des sources à compiler chez l'utilisateur (à l'installation). Le déterminisme est la propriété d'un build reproductible : deux builds des mêmes sources donnent les mêmes octets, ce qui permet de ne sauvegarder qu'un diff.

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, SSD NVMe, Linux 7.0. Rust 1.99, compilation portable (sans `target-cpu=native`), `lto = "thin"`. Mesures du 2026-11-16. Pas de GDAL ni de pyproj sur la machine : tout est lu en Rust (SQLite compilé avec `rusqlite`, archives 7z avec `sevenz-rust2`, Parquet avec `parquet`), et les projections sont écrites à la main, puis validées contre `cs2cs`.

## Méthode

### Sources (B-06)

| Source | Édition | Fichier | Taille | Licence |
|---|---|---|---|---|
| BD TOPO, Manche (D050), GeoPackage | 3.5, 2026-09-15 | `data/sources/bdtopo-d050-2026-09-15/` | 434 Mo (7z), 2,55 Go décompressé | Licence Ouverte Etalab (version 2.0 à vérifier) |
| BD TOPO, Paris (D075) | 3.5, 2026-09-15 | `data/sources/bdtopo-d075-2026-09-15/` (P-01) | 1,41 Go | idem |
| RGE ALTI 1 m, Manche, ASC | 2.0, 2022-12-21 (livraison 2024-03) | `data/sources/rgealti-d050-1m-2022-12-21/` | 3,68 Go (7z), 6 701 dalles, 56 Go décompressé | Licence Ouverte Etalab (à vérifier) |
| RGE ALTI 5 m, Manche, ASC | 2.0, 2022-12-21 | `data/sources/rgealti-d050-5m-2022-12-21/` | 289 Mo (7z) | idem |
| INSEE, Filosofi 2021, carreaux de 200 m (métropole) | 2021, publié le 2026-02-16 | `data/sources/insee-filosofi2021-carreaux-2026-11-16/` (Parquet de data.gouv.fr) | 95 Mo | Licence Ouverte 2.0 (data.gouv.fr) |
| INSEE, Filosofi 2021, carreaux de 1 km (métropole, Martinique, Réunion) | idem | idem (CSV de l'INSEE) | 17 Mo | idem |

Les sommes MD5 fournies par l'IGN sont vérifiées. Le serveur de l'INSEE coupe le fichier de 200 m (91 Mo) à chaque essai et refuse les reprises : la même table a été prise sur data.gouv.fr en Parquet. Les tailles des sources de **toute la France** viennent du service de téléchargement de la Géoplateforme (métadonnées seulement, [outils/tailles_sources.py](outils/tailles_sources.py), [tailles-sources.csv](tailles-sources.csv)).

Trois découvertes sur les sources :

- **L'extraction départementale de la BD TOPO a une zone tampon d'environ 5 km** : le fichier D050 contient 532 communes, dont 87 des départements voisins (14, 35, 53, 61). C'est ce qui rend les raccords possibles (voir *Inventaire*).
- **Les carreaux de l'INSEE ne sont pas sur le Lambert-93** depuis Filosofi 2017 : ils sont sur la grille européenne INSPIRE, en ETRS89-LAEA (EPSG 3035, identifiants `CRS3035RES200mN…E…`). Un carreau est donc à cheval sur plusieurs tiles, contrairement à ce que supposait A-01 (« comme les carreaux de l'INSEE »).
- **L'IGN ne couvre pas le Pacifique** : la BD TOPO et le RGE ALTI existent pour les DROM, Saint-Pierre-et-Miquelon, Saint-Martin et Saint-Barthélemy, mais pas pour la Nouvelle-Calédonie, la Polynésie française ni Wallis-et-Futuna, qui ont leurs propres services géographiques. Décision prise pendant le prototype : le Pacifique sort du jeu (voir *Décisions*). En Guadeloupe, le RGE ALTI est en WGS84 / UTM 20 et la BD TOPO en RGAF09 / UTM 20 (à concilier, à vérifier).

### Les étapes de la chaîne ([src/chaine.rs](src/chaine.rs))

| Étape | Ce qu'elle fait | Parallélisme |
|---|---|---|
| 1. Lecture ([src/objets.rs](src/objets.rs), [src/gpkg.rs](src/gpkg.rs)) | 17 couches de la BD TOPO, décodées du WKB, triées par `fid` ; énumérations converties en octets selon des listes fixes | une connexion SQLite par couche |
| 2. Inventaire | tiles du département : centre dans une de ses communes, ou en mer à moins de 30 km de sa terre la plus proche (carte des départements à 100 m, distance de chanfrein) ; plus un **halo** d'une tile, pris dans la zone tampon | — |
| 3. Population ([src/population.rs](src/population.rs)) | carreaux de 200 m → habitants par bâtiment | — |
| 4. Répartition | objets rangés par tile (boîte englobante) ; routes et voies ferrées **coupées aux bords** ; ponts ; mairies | découpe en parallèle |
| 5. Relief ([src/relief.rs](src/relief.rs)) | dalles ASC lues **en flux** dans l'archive 7z, sans rien écrire sur disque ; un bloc LZMA2 par tâche (427 blocs) | par bloc |
| 6. Passe 1 | par tile : bâti à 1 m, franchissable à 2 m, communes, occupation du sol, composantes de la zone, encodage des couches | par tile |
| 7. Fusion ([src/zone.rs](src/zone.rs)) | zone accessible : union-find sur les bords des tiles | — |
| 8. Passe 2 | par tile : cases hors zone, cours fermées et passages percés, relief raccordé, assemblage, empreinte | par tile |
| 9. Écriture ([src/tuile.rs](src/tuile.rs)) | paquets de 16 × 16 tiles, fichier global des communes | — |

### Le contenu d'une tile

Une tile est un en-tête, une table de sections compressées séparément en zstd (pour ne décoder que ce dont une vue a besoin, C-04), et une empreinte BLAKE3. Les coordonnées vectorielles sont locales, centrées sur la tile (C-01), quantifiées au 1/16 m, en différences et entiers variables.

| Section | Contenu | Résolution, format |
|---|---|---|
| En-tête | zone de projection, (ix, iy), drapeaux terre / mer / relief, **part de bâti** (P-02 : distance de vue), habitants, nombre de bâtiments | 30 octets, non compressé |
| `relief` | altitude des **nœuds** aux multiples de 2 m, **bords compris** (501 × 501) : une tile se maille seule | quantifiée à 10 cm, prédiction MED (celle de JPEG-LS), résidus sur deux plans d'octets |
| `bati` | bâtiments **possédés** par la tile (centroïde dedans) : identifiant BD TOPO, nature, usages, construction légère, hauteur, étages, logements, emprise complète (même si elle déborde) | vecteur |
| `franchissable` | grille de 2 m, règle de la majorité (P-03), moins les **murs** (BD TOPO, « Mur », « Mur anti-bruit »), plus les **passages percés** | bitmap |
| `zone` | cases franchissables **hors zone accessible** (P-04), et liste des cases percées (B-02) | bitmap s'il y en a, sinon un octet |
| `reseau` | morceaux de routes (nature, importance, position par rapport au sol, largeur, voies, sens, accès, vitesse, restrictions de poids, de hauteur, de largeur ; altitude des sommets sur ouvrage) et de voies ferrées (électrification, voies), identifiant et rang du morceau | vecteur, coupé aux bords |
| `ponts` | ouvrages « Pont » : longueur, largeur et importance de la route portée, restriction de poids, ce qui est franchi (eau, route, voie ferrée), routes et voies portées, géométrie | vecteur |
| `occupation` | par case de 2 m : classe de végétation (forêts de feuillus, de conifères, mixte, bois, haie, lande, verger, vigne, peupleraie, forêt ouverte), eau douce, eau salée, estran, mer | un octet par case |
| `communes` | **fragments tile × commune** (A-19) : code INSEE, surface, habitants ; carte des fragments à 2 m si la tile en a plusieurs | vecteur + octets |
| `lieux` | lieux d'intérêt possédés (voir *Lieux*) : type, identifiant, nom, géométrie | vecteur |

Hors des tiles, un fichier par département, `communes.bin` (11 ko pour la Manche) : code, nom, bâtiment de la mairie, communes voisines (graphe d'adjacence de [mairies.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/locations/mairies.md)).

**Mer** : une case est en mer si elle n'est dans aucune commune. Les tiles de mer à moins de 30 km de la côte (G-01) n'ont qu'un en-tête et une occupation uniforme.

### Raccords

- **Relief** : chaque nœud a une seule source, la dalle de 1 m qui le contient. Les nœuds du bord est et du bord sud viennent des dalles voisines (le centre d'un pixel ASC est sur un mètre entier, bord nord et bord ouest de la dalle inclus). Deux tiles voisines ont donc exactement les mêmes valeurs sur leur bord commun. Contrôle : comparaison de tous les nœuds de bord de toutes les paires de tiles voisines.
- **Bâtiments** : chaque bâtiment appartient à la tile qui contient son centroïde, et à elle seule. Sa rastérisation, elle, va dans toutes les tiles qu'il couvre, avec la règle du centre de case, qui ne dépend que des coordonnées globales (test `raster_partage_exact_entre_tiles`).
- **Routes** : chaque tronçon est coupé aux lignes de la grille ; les points de coupe sont calculés une fois, en coordonnées globales ; chaque sous-segment va à la tile de son milieu. Le morceau garde l'identifiant du tronçon et son rang, pour recoudre le graphe de P-09.
- **Entre départements** : une tile appartient au département qui contient son centre. La zone tampon de 5 km de l'IGN fournit tout ce qui déborde (bâtiments des voisins, halo de la zone accessible). Un carreau INSEE frontalier est réparti de la même façon par les deux départements, puisqu'ils ont le même bâti dans leur zone tampon.

### Zone accessible, cours fermées, passages ([src/zone.rs](src/zone.rs))

P-04 calculait la zone en une seule passe, avec un tableau d'étiquettes pour tout un bloc. À l'échelle d'un département (10⁹ cases de 2 m) ou de la France (10¹¹), ce tableau ne tient plus en mémoire. La chaîne fait donc l'**étiquetage distribué** classique :

1. chaque tile étiquette ses composantes et ne garde que les étiquettes de ses quatre bords ;
2. une union-find relie les composantes à travers les bords ;
3. chaque tile réétiquette (même résultat) et marque ses cases hors zone.

Analogie : un routage par zones où seuls les routeurs de frontière échangent leurs annonces. La 4-connexité est celle de P-04 (8-connexité sans couper les coins).

**Critère d'accès** (changé par rapport à P-04) : une composante est accessible si une route ou un chemin non souterrain la traverse, ou si c'est la plus grande. Sinon, les îles (Chausey, Tatihou) et les îlots coupés par la mer n'auraient aucun zombie.

**Passages** (B-02) : pour chaque cour fermée d'au moins 4 cases (16 m²), le plus court chemin à travers le bâti et les murs (jamais la mer) jusqu'à la zone, d'au plus 30 cases (60 m) ; ses cases deviennent franchissables. C'est un parcours en largeur dans la tile, dans un ordre fixe.

### Population ([src/population.rs](src/population.rs))

Répartition **dasymétrique** : les habitants d'un carreau de 200 m vont à ses bâtiments résidentiels (usage résidentiel ou logements), au prorata de leur plancher (P-05 : étages, sinon hauteur / 3 m) ; à défaut, à ses autres bâtiments, sauf les annexes et les constructions légères ; à défaut, à 100 points répartis sur le carreau. Chaque carreau porte un entier (`ind` arrondi), réparti par la **méthode du plus fort reste**, égalités départagées par l'identifiant du bâtiment : la somme est exacte. Les habitants d'un bâtiment vont au fragment qui contient son centroïde. Le nombre de zombies se déduit des habitants à la création de la partie (moins les évacués, A-10) : il n'est pas stocké.

### Projections ([src/geo.rs](src/geo.rs))

Lambert-93 copié de P-06 ; azimutale équivalente de Lambert (EPSG 3035, formules ellipsoïdales de Snyder) écrite pour les carreaux. Validées contre `cs2cs` sur 200 points de la France ([resultats-geo.csv](resultats-geo.csv)).

### Déterminisme

- Les phases parallèles ne travaillent que sur leur tile, à partir de listes triées. Les étapes globales (fusion, population, mairies) parcourent des structures triées. L'union-find garde la plus petite racine, ce qui la rend indépendante de l'ordre des unions.
- L'altitude est lue en centimètres entiers, sans flottant.
- Le niveau de zstd et la version de la bibliothèque sont fixés.
- Contrôle : empreinte BLAKE3 de chaque tile, empreinte globale de la liste des tiles et du fichier des communes, et `diff` des paquets, octet par octet, entre exécutions.

### Chargement en jeu ([src/charge.rs](src/charge.rs))

Pour chaque tile, sur un thread, page cache chaud : lecture (`pread` dans le paquet), vérification de l'empreinte, décompression et décodage de chaque section. S'y ajoute ce que le jeu recalcule au lieu de le stocker : le bâti à 1 m (vue de P-01 à P-04), rastérisé depuis les emprises de la tile et de ses voisines ; la capacité de chaque bâtiment (P-05) ; les arbres (B-04), tirés par hachage dans les cases boisées. Le stockage du bâti de 1 m et des arbres est mesuré en regard.

### Scénarios ([run.sh](run.sh))

| Scénario | Mesure |
|---|---|
| 1. Manche de bout en bout (relief 2 m, 10 cm, zstd 9) | temps par étape, mémoire, tailles, contrôles, chargement de chaque tile |
| 2. Tiles typiques | centre de Cherbourg, Saint-Lô, bocage, côte, baie du Mont-Saint-Michel, La Hague, Flamanville, Granville, mer ([images/](images)) |
| 3. Paris | même chaîne sur la BD TOPO de Paris, sans relief (pas de RGE ALTI de Paris dans les sources), comparée aux tiles de P-01 |
| 4. Déterminisme | deux exécutions à 16 threads, puis 8, 4 et 1 thread : mêmes octets |
| 5. Variantes | relief à 1, 2 et 5 m, quantifié à 1 ou 10 cm, et 5 m natif de l'IGN (toutes les tiles) ; chaîne sans les murs ; niveaux de zstd 1, 3, 9 et 19 |
| 6. Extrapolation ([outils/extrapoler.py](outils/extrapoler.py)) | France entière à partir des types de tiles |

**Types de tiles** (classement par habitants INSEE, pour l'extrapolation) : `mer` (aucune terre), `cote` (terre et mer, sans habitant ni bâti), `vide` (0 habitant), `rural` (1 à 99), `bourg` (100 à 999), `urbain` (1 000 à 4 999), `dense` (5 000 et plus).

### Paramètres de prototype

Ce sont des **hypothèses de mesure**, pas des statistiques de jeu : relief à 2 m et 10 cm ; zstd niveau 9 ; cour fermée à percer à partir de 16 m², passage de 60 m au plus ; mer à 30 km des côtes ; murs rastérisés sur 1,5 m de part et d'autre ; haies de 3 m de large faute de largeur ; cours d'eau de 2 m (15 m pour les classes larges) ; densité d'arbres d'un arbre pour 25 m² en forêt fermée (1/33 en bois, 1/16 en haie, moins en lande et en verger) ; capacité de 35 m² par colon et de 20 m² par poste (P-05).

Reproduire : `./run.sh` (environ 45 min) ; `ESSAI=1 ./run.sh` valide toutes les commandes en 2 min (six tiles, relief à 5 m). Images seules : `./run.sh images`. Tests : `cargo test --release` (6 tests) ; micro-bancs : `cargo test --release banc -- --ignored --nocapture`. Tailles des sources nationales : `python3 -I outils/tailles_sources.py > tailles-sources.csv`.

## Budgets

Il n'y a pas de budget par tick ici. Les références sont celles d'une installation et d'un chargement en jeu :

- **chaîne** : une machine de joueur (la machine de référence), 32 Go de mémoire, quelques heures au plus pour la France entière si elle tourne à l'installation ;
- **chargement d'une tile** : P-04 étale l'activation d'une tile sur environ 27 ticks, sans jamais dépasser 3 ms par tick ; le chargement doit pouvoir s'étaler de même.

## Mesures

### 1. La Manche de bout en bout ([resultats-etapes.csv](resultats-etapes.csv), [resultats-controles.txt](resultats-controles.txt))

**14 076 tiles** du département, dont 6 268 avec de la terre et 7 808 de mer à 30 km, plus 787 tiles de halo. Sortie : **416 Mo** en 81 paquets. Durée : **66 s** sur 16 threads.

| Étape | Durée (16 threads) | Mémoire maximale après l'étape |
|---|---|---|
| Lecture de la BD TOPO (17 couches, 680 000 bâtiments, 227 000 tronçons, 902 000 haies) | 1,7 s | 2,2 Go |
| Inventaire | 0,06 s | 2,2 Go |
| Population (lecture du Parquet national : 2,3 millions de carreaux) | 8,4 s | 2,2 Go |
| Répartition, découpe des routes, ponts, mairies | 0,5 s | 2,2 Go |
| Relief : 6 701 dalles, 42,7 Go d'ASC décompressés en flux | 31,5 s | 5,7 Go |
| Passe 1 | 8,1 s | 10,8 Go |
| Fusion de la zone | 0,04 s | 10,8 Go |
| Passe 2 | 15,1 s | 10,9 Go |
| Écriture | 0,7 s | 10,9 Go |

- **Place disque temporaire** : la BD TOPO doit être décompressée, car SQLite a besoin d'un accès direct au fichier : 2,55 Go pour la Manche, en 4,0 s avec `7z` ([resultats-decompression.txt](resultats-decompression.txt)). Le relief, lu en flux, n'en demande **aucune**, au lieu de 56 Go décompressés.
- **Mémoire** : 10,9 Go au maximum. Elle croît avec le nombre de tiles traitées d'un coup (les résultats de la passe 1 restent en mémoire jusqu'à la fusion) : 16 Go dans une première version qui gardait aussi les sections du halo et les sections déjà assemblées.
- **Analyse d'une dalle ASC** : 8,3 ms, soit 818 Mo/s ([banc.txt](banc.txt)). Le relief est limité par la décompression LZMA.

### 2. Nombre de threads et déterminisme ([resultats-determinisme.csv](resultats-determinisme.csv))

| Threads | Durée | Mémoire max | Empreinte |
|---|---|---|---|
| 16 | 64,8 s | 10,9 Go | `d929a821…` |
| 16 (2ᵉ exécution) | 65,8 s | 10,9 Go | `d929a821…` |
| 8 | 79,0 s | 10,7 Go | `d929a821…` |
| 4 | 140,3 s | 10,6 Go | `d929a821…` |
| 1 | 503,6 s | 10,5 Go | `d929a821…` |

Les **paquets sont identiques octet pour octet** (`diff -r`) dans les cinq exécutions, et à la première campagne, qui mesurait les variantes du relief. Sur un thread, le relief prend 318 s ; sur 16, 31 s (gain de 10) ; l'ensemble passe de 504 à 65 s (gain de 7,7). De 8 à 16 threads, le gain n'est que de 18 % (SMT, comme P-03).

### 3. Taille des tiles par type ([resultats-tiles-manche.csv](resultats-tiles-manche.csv), [resultats-tiles-paris.csv](resultats-tiles-paris.csv))

Taille compressée d'une tile entière, en ko :

| Type | Tiles | Médiane | p90 | Max | Non compressée (médiane) |
|---|---|---|---|---|---|
| mer | 7 808 | 0,2 | 0,2 | 76 | 281 |
| côte | 176 | 57,6 | 85,1 | 115 | 847 |
| vide | 252 | 58,8 | 84,5 | 108 | 1 065 |
| rural | 4 888 | 60,4 | 79,9 | 121 | 1 067 |
| bourg | 892 | 70,9 | 103,9 | 134 | 891 |
| urbain | 58 | 118,1 | 149,1 | 177 | 914 |
| dense (Manche) | 2 | 209,7 | 209,7 | 210 | 981 |
| dense (Paris, **sans relief**) | 91 | 77,3 | 106,5 | 126 | 628 |

Par couche, compressée, en ko (médiane / p90) :

| Couche | vide | rural | bourg | urbain | dense (Manche) | dense (Paris) |
|---|---|---|---|---|---|---|
| relief | 50,3 / 74,8 | 47,8 / 67,7 | 50,3 / 80,1 | 73,4 / 93,2 | 91,5 | — |
| bâti | 0,1 / 0,4 | 1,2 / 2,2 | 4,2 / 10,5 | 24,0 / 44,9 | 90,6 | 47,0 / 75,4 |
| franchissable | 0,1 / 0,4 | 0,7 / 1,2 | 2,1 / 4,4 | 8,9 / 12,5 | 16,6 | 14,2 / 18,1 |
| zone | 0,0 / 0,0 | 0,0 / 0,0 | 0,0 / 0,1 | 0,1 / 0,3 | 1,0 | 1,9 / 3,9 |
| réseau | 0,6 / 1,3 | 1,0 / 1,5 | 1,7 / 3,2 | 5,8 / 7,2 | 7,4 | 7,0 / 9,1 |
| ponts | 0 / 0 | 0 / 0,1 | 0 / 0,1 | 0 / 0,2 | 0,1 | 0,1 / 0,5 |
| occupation | 5,7 / 9,5 | 8,5 / 11,2 | 8,5 / 11,2 | 5,0 / 7,4 | 6,1 | 5,9 / 8,3 |
| communes | 0,5 / 1,5 | 0,4 / 1,4 | 0,1 / 1,2 | 0,0 / 0,7 | 0,0 | 0,6 / 1,5 |
| lieux | 0 / 0 | 0 / 0 | 0 / 0,1 | 0 / 0,1 | 0 | 0 / 0,2 |

**Le relief fait 80 % d'une tile rurale.** Le bâti ne domine qu'en ville.

### 4. Relief : résolution et quantification ([resultats-relief.csv](resultats-relief.csv))

Sur les 6 238 tiles terrestres (au moins la moitié des nœuds renseignés), erreur mesurée en chaque nœud de 1 m par interpolation bilinéaire :

| Variante | Taille médiane | p90 | Max | Total Manche | Erreur RMS (médiane / p90) | Erreur max (médiane / p90 / p99) |
|---|---|---|---|---|---|---|
| 1 m, 1 cm (sans perte) | 152 ko | 530 ko | 686 ko | 1,51 Go | 0 | 0 |
| 1 m, 10 cm | 150 ko | 225 ko | 327 ko | 1,01 Go | 0 / 2,9 cm | 0 / 5 / 5 cm |
| 2 m, 1 cm | 49 ko | 154 ko | 204 ko | 464 Mo | 3,0 / 3,8 cm | 13 cm / 1,3 m / 4,4 m |
| **2 m, 10 cm** | **48 ko** | **72 ko** | **108 ko** | **327 Mo** | **3,0 / 4,3 cm** | **13 cm / 1,3 m / 4,4 m** |
| 5 m, 10 cm | 12 ko | 16 ko | 24 ko | 77 Mo | 3,9 / 9,8 cm | 37 cm / 2,1 m / 6,5 m |
| 5 m natif (IGN) | 12 ko | 30 ko | 41 ko | 102 Mo | 3,9 / 9,7 cm | 37 cm / 2,2 m / 6,8 m |

- Dans la moitié des tiles, les valeurs du RGE ALTI sont **déjà au décimètre** : quantifier à 10 cm n'y perd rien. Dans les autres, 1 cm de bruit coûte jusqu'à deux fois la taille.
- Les erreurs maximales de plusieurs mètres sont des **ruptures de pente** : falaises du Cotentin, carrières, digues, talus. Le 2 m en garde la forme, le 5 m les adoucit sur 5 m.
- Le **5 m natif** de l'IGN n'apporte rien de plus que le 5 m tiré du 1 m : même erreur, plus gros (il est livré au centimètre).
- Chaîne complète par variante : 1 m → 1,11 Go, 98 s, **20,5 Go de mémoire** ; 2 m → 416 Mo, 66 s, 10,9 Go ; 5 m → 163 Mo, 53 s, 7,8 Go.
- Décodage d'une grille ([banc.txt](banc.txt)) : 4,3 ms à 1 m, 1,0 ms à 2 m, 0,17 ms à 5 m.

### 5. Raccords et contrôles ([resultats-controles.txt](resultats-controles.txt))

| Contrôle | Résultat |
|---|---|
| Relief : marches entre tiles voisines | **0** sur 6 339 654 nœuds de bord (12 654 paires) |
| Bâtiments : chaque bâtiment dans une seule tile | 593 511 dans les tiles du département, 17 835 dans le halo, 68 055 au-delà : somme exacte (679 401), **0 doublon** |
| Routes : longueur des morceaux contre longueur des tronçons | 36 517,121 km des deux côtés (écart relatif de 6 × 10⁻¹⁶) |
| Population : somme des fragments = total | **exacte** (490 154 habitants dans les tiles du département, autant dans leurs fragments) |
| Projections contre PROJ | Lambert-93 0,066 mm ; LAEA 0,070 mm ; LAEA → Lambert-93 0,066 mm (sortie de `cs2cs` au 0,1 mm) |
| Part de bâti contre P-01 (Paris, mêmes tiles) | identique sur les 91 tiles communes (écart max de 5 × 10⁻⁵, l'arrondi du CSV de P-01) |
| Décodage | toutes les images sont relues depuis les paquets écrits ; empreinte vérifiée à chaque chargement |
| Tests | 6 tests ([tests.txt](tests.txt)) : découpe sans perte, rastérisation partagée exacte, relief aller-retour, centimètres exacts, plus fort reste, union-find indépendante de l'ordre |

### 6. Population et zombies initiaux

| | Manche | Paris |
|---|---|---|
| Habitants INSEE dans les tiles | 490 154 | 2 032 166 |
| Population légale de la BD TOPO | 497 522 | 2 103 778 |
| Écart | −1,5 % | −3,4 % |
| Habitants posés sur des points (carreau sans bâtiment) | 4 948 (1,0 %) | 31 |
| Tiles à plus d'un fragment de commune | 3 686 (59 % des tiles terrestres), jusqu'à 5 | 79 sur 105, jusqu'à 5 |
| Tile la plus peuplée | 6 648 | **42 581** |

L'écart avec la population légale vient vraisemblablement du champ de Filosofi (ménages fiscaux, sans la population des communautés : à vérifier). En France, 69 carreaux de 1 km dépassent 20 000 habitants et **2 dépassent 40 000**, la saturation de la migration (P-04). À Paris, 50 tiles sur 105 dépassent 20 000 habitants, le chiffre que P-01 à P-04 prenaient pour une tile dense.

### 7. Zone accessible, cours fermées, murs

| | Manche | Paris |
|---|---|---|
| Composantes, dont accessibles | 6 624, dont 180 | 56 863, dont 476 |
| Cases libres accessibles (avant percement) | 99,98 % | 93,9 % |
| Cours fermées d'au moins 16 m² | 1 865 | 32 721 |
| Dont percées (passage de 60 m au plus) | 1 654 (89 %) | 32 670 (99,8 %) |
| Cases de 2 m percées | 3 799 | 165 697 (0,66 km² de bâti) |
| Cases restées hors zone, rapportées à la zone | 0,01 % | 0,18 % |

Sans les murs, la Manche a 6 330 composantes au lieu de 6 624, et la zone passe de 99,984 % à 99,990 % : les murs de la BD TOPO ferment peu de cours. En rural, la section `zone` est presque toujours vide (un octet).

### 8. Ponts (B-03) et réseau

Sur les 3 292 ouvrages « Pont » de la Manche :

- **restriction de poids renseignée : 1** (et 37 tronçons de route sur 195 784, dont 1 sur ouvrage) ;
- largeur de la route portée connue : 1 755 (53 %) ;
- importance de la route portée : 1 ou 2 pour 279, 3 ou 4 pour 958, 5 ou 6 pour 1 354 ; 701 ponts ne portent aucun tronçon (passerelles, ouvrages sans route) ;
- ce qui est franchi (heuristique : objet au sol à moins de 3 m) : eau pour 2 711, voie ferrée pour 232 ; « route » est surestimé, car les rampes d'accès au sol touchent le pont ;
- 154 ponts portent une voie ferrée ;
- 2 724 tronçons sont sur ouvrage (position par rapport au sol ≥ 1).

### 9. Lieux d'intérêt de LOCATIONS dans la Manche ([resultats-lieux.csv](resultats-lieux.csv))

| Type (LOCATIONS) | Trouvé dans la BD TOPO | Couche, attribut | Remarque |
|---|---|---|---|
| CNPE | Flamanville | `zone_d_activite_ou_d_interet`, « Centrale électrique » | 7 « centrales électriques » sans détail, dont 6 petites : filtrer par nom (à vérifier) |
| Usine de retraitement | La Hague | idem, « Usine », toponyme | pas un type de LOCATIONS |
| Ports | 28 (Cherbourg, Granville, Carteret, Diélette, Barfleur…) + 3 gares maritimes | `equipement_de_transport`, « Port » | type (pêche, plaisance) dans `nature_detaillee` ; **tirant d'eau absent** (SHOM, B-06) |
| Gares | 6 | idem, « Gare voyageurs… » | |
| Aérodromes | 5 (Cherbourg-Maupertus LFRC, Granville, Avranches…) + 16 pistes | `aerodrome`, `piste_d_aerodrome` | longueur de piste à calculer depuis la géométrie (pas d'attribut) |
| Barrages | 212 | `construction_lineaire` et `surfacique`, « Barrage » | ce sont surtout des seuils ; aucun des 10 grands barrages n'est dans la Manche (à vérifier) |
| Enceintes militaires | 15 (base aéronavale de Maupertus…) | `zone_d_activite_ou_d_interet` | |
| Mairies | 445 communes : 444 mairies trouvées, 1 commune sans mairie (plus grand bâtiment) | idem, « Mairie » sans détail ou « Hôtel de ville » | les 227 « mairies de commune déléguée » sont écartées |
| CTM de Rosnay | — | — | dans l'Indre |
| Poches de survivants | — | — | liste écrite à la main |

**Paris** : la mairie de chaque arrondissement porte le code INSEE de Paris (75056), pas celui de l'arrondissement : 19 arrondissements sur 20 n'ont pas trouvé leur mairie par le code. Il faut un rattachement géométrique.

### 10. Chargement en jeu ([resultats-charge-resume.csv](resultats-charge-resume.csv), [resultats-charge.csv](resultats-charge.csv), [resultats-niveaux.csv](resultats-niveaux.csv))

Un thread, page cache chaud, en ms (médiane / max) :

| Étape | rural | bourg | urbain | dense (Manche) | dense (Paris, sans relief) |
|---|---|---|---|---|---|
| Lecture du paquet | 0,015 / 0,05 | 0,017 / 0,05 | 0,021 / 0,03 | 0,03 | 0,02 / 0,05 |
| Relief (zstd + MED) | 1,93 / 2,71 | 2,02 / 2,56 | 2,22 / 2,55 | 2,28 | — |
| Bâti à 1 m, **recalculé** | 0,09 / 0,71 | 0,26 / 1,79 | 1,37 / 2,91 | 4,58 | 3,47 / 5,69 |
| Arbres, **recalculés** | 0,50 / 1,36 | 0,49 / 1,05 | 0,39 / 0,85 | 0,44 | 0,42 / 1,00 |
| **Total** | **2,7 / 3,8** | **3,0 / 5,2** | **4,4 / 6,5** | **8,2** | **4,5 / 6,9** |

- L'en-tête seul (vue simplifiée, tiles inertes) se lit en 0,02 ms, sans rien décompresser.
- Stocker le bâti de 1 m coûterait 1,3 ko (rural), 18 ko (urbain), 33 à 50 ko (Paris dense) de plus par tile, pour gagner 0,1 à 5 ms.
- Une tile rurale compte 8 000 arbres en médiane (38 000 au plus) : les stocker coûterait 33 ko par tile, plus que tout le reste hors relief, pour gagner 0,5 ms.

**Niveaux de zstd**, sur une tile sur dix (1 408 tiles), un thread :

| Niveau | Taille | Compression | Décompression |
|---|---|---|---|
| 1 | 47,1 Mo | 0,7 s | 0,32 s |
| 3 | 46,8 Mo | 0,9 s | 0,30 s |
| 9 | 41,4 Mo | 6,1 s | 0,25 s |
| 19 | **35,1 Mo** (−15 %) | 175 s | 0,23 s |

### 11. Extrapolation à la France entière ([resultats-extrapolation.csv](resultats-extrapolation.csv), [resultats-extrapolation.txt](resultats-extrapolation.txt))

Nombre de tiles par type en métropole : carreaux de 1 km de l'INSEE par classe d'habitants (374 511 habités, 62,6 millions d'habitants), et surface terrestre restante pour les tiles vides (543 940 km², à vérifier). Outre-mer hors Pacifique : 89 500 km², répartis comme la Manche (hypothèse grossière : la Guyane est surtout de la forêt vide). Mer à 30 km : tiles de mer de la Manche par km de côte, rapportées à 7 500 km de côte (à vérifier). Le relief et la lecture sont rapportés au volume des sources nationales.

| Périmètre | Tiles | Sortie (zstd 9, relief 2 m) |
|---|---|---|
| Métropole | 543 940 | 34,6 Go |
| Outre-mer (DROM, Saint-Pierre-et-Miquelon, Saint-Martin, Saint-Barthélemy) | 89 496 | 5,8 Go |
| Mer à 30 km | 167 314 | 0,2 Go |
| **Total** | **800 750** | **40,6 Go** (environ 35 Go en zstd 19 ; environ 16 Go avec un relief de 5 m) |

| | Avant distribution | À l'installation |
|---|---|---|
| Téléchargement | **35 à 41 Go** de tiles | **594 Go** (BD TOPO 32,4 Go + RGE ALTI 1 m 561 Go), ou 71 Go en se contentant d'un relief de 5 m (RGE ALTI 5 m : 38,9 Go) |
| Durée de calcul | — (chez le projet) | **1,9 h** sur la machine de référence, dont 1,3 h de décompression du relief ; plus le téléchargement |
| Mémoire | — | 11 Go par département de la taille de la Manche ; 20 Go pour un relief de 1 m |
| Place disque temporaire | — | la BD TOPO d'un département décompressée (2,5 Go pour la Manche) si l'on traite un département à la fois ; 0 pour le relief |

## Analyse

**1. B-01 : avant distribution, sans hésiter.** Le facteur décisif est le relief. Pour obtenir un relief de 2 m, il faut le RGE ALTI de 1 m : **561 Go** d'archives pour la France, quinze fois les tiles qu'on en tire. Personne ne télécharge 600 Go pour installer un jeu. Avec le RGE ALTI de 5 m, le téléchargement tombe à 71 Go, mais pour un relief moins précis et un fichier encore plus gros que les tiles finales. Dans les deux cas, l'installation demanderait en plus deux heures de calcul et 11 Go de mémoire, sur une machine qui n'est pas celle de tous les joueurs. Avant distribution, le joueur télécharge 35 à 41 Go, une fois, comme un gros jeu du commerce. Analogie : on livre un binaire signé, pas des sources à compiler chez le client, d'autant moins quand les sources pèsent quinze fois le binaire.

**2. La chaîne elle-même est rapide.** Deux heures pour la France sur la machine de référence. Le relief en prend 70 % : c'est de la décompression LZMA, déjà parallèle par bloc. Le reste (lecture, rastérisation, zone, encodage) tient en 30 min. La chaîne peut tourner dans l'intégration continue du projet à chaque édition de la BD TOPO (trimestrielle), sur une machine ordinaire.

**3. Le relief décide de la taille ; le 2 m est le bon compromis.** Il fait 80 % d'une tile rurale. Le 1 m triple la taille pour 3 cm d'erreur RMS en moins, et double la mémoire de la chaîne. Le 5 m divise la taille par quatre, mais il efface les ruptures de pente sur 5 m : talus de bocage, digues, bords de falaise, quais, exactement ce que voit un soldat en vue rapprochée et ce que les flow fields (2 m, P-03) et le génie (B-05) devront suivre. Le 2 m est aussi la grille des flow fields, du franchissable et de la zone : une case, un nœud. Quantifier à 10 cm ne perd rien dans la moitié des tiles, où les données sont déjà au décimètre, et divise par deux la taille ailleurs.

**4. Ce qui se précalcule : tout ce qui demande une vue d'ensemble ; le reste se recalcule.** Le critère est celui de P-04 (zone accessible) : un calcul qui a besoin des voisins, ou de tout le territoire, se fait dans la chaîne ; un calcul local et rapide se fait au chargement, s'il coûte moins de quelques millisecondes étalables.

| Donnée | Choix | Pourquoi |
|---|---|---|
| Zone accessible, cours fermées, passages | **précalculés** | globaux (union-find sur tout le territoire) ; quasi gratuits en rural |
| Habitants par fragment, mairies, voisinage des communes | **précalculés** | globaux (carreaux INSEE, toutes les communes) |
| Part de bâti | **précalculée** (en-tête) | sert sans rien décompresser (P-02) |
| Franchissable de 2 m | **stocké** | il intègre les murs et les passages ; 0,7 à 17 ko |
| Relief | **stocké** | sa source n'est pas distribuable à cette taille |
| Bâti de 1 m | **recalculé** | 0,1 à 5 ms depuis les emprises, contre 1 à 50 ko par tile |
| Capacité des bâtiments | **recalculée** | quelques microsecondes |
| Arbres | **recalculés** | 0,5 ms, contre 33 ko par tile rurale |
| Terrain des flow fields | **recalculé** | P-04 |

**5. Le chargement d'une tile est court, et étalable comme l'activation.** 3 ms pour une tile rurale, 5 à 8 ms pour une tile dense, sur un thread. C'est au-dessus des 3 ms par tick de P-04 : il faut l'étaler comme l'activation (relief et bâti dans deux ticks différents, en tâche de fond). Le premier poste est le **décodage du relief** (2 ms) : la prédiction MED est séquentielle, mais elle se découpe facilement par bandes de lignes indépendantes, au prix de quelques octets. Pour le streaming de C-04, une tile fait 60 ko en moyenne : la caméra qui survole la France à 1 km/s en charge une ou deux par seconde à basse altitude, soit 100 ko/s, rien pour un SSD. La vue simplifiée n'a besoin que des en-têtes.

**6. Le déterminisme ne coûte rien, et il est vérifié de bout en bout.** Mêmes octets à 1, 4, 8 et 16 threads, et sur deux campagnes. Trois règles y suffisent, les mêmes que pour P-04 : chaque tâche ne travaille que sur sa tile, les étapes globales parcourent des structures triées, et l'arithmétique sensible est entière (altitudes en centimètres, plus fort reste). C'est la condition de A-13 : le monde de base se régénère à l'identique, et une sauvegarde peut ne garder que la différence. Une réserve : les projections des carreaux utilisent la bibliothèque mathématique du système ; pour garantir les mêmes octets sous Windows, il faut une bibliothèque mathématique en Rust pur (P-18).

**7. Les raccords sont exacts, et l'extraction de l'IGN les rend possibles par département.** Aucune marche sur 6,3 millions de nœuds de bord, aucun bâtiment en double, longueur des routes conservée au 10⁻¹⁵. La zone tampon de 5 km de chaque département sert de halo : on peut traiter les départements un par un (mémoire bornée, parallélisme par département sur plusieurs machines), sans les recoudre ensuite, tant que la règle d'appartenance (centre de la tile) et le traitement des carreaux frontaliers sont les mêmes partout.

**8. La population réelle est plus concentrée que ce que supposaient P-01 à P-04.** Les tiles parisiennes ont 21 000 habitants en médiane et jusqu'à 42 600, au-delà de la saturation de migration (40 000) mais sous la capacité d'une tile active (65 000). Moins les évacués (environ 10 %), la tile la plus dense compterait environ 38 000 zombies, le double du pire cas de P-01. Ailleurs, rien à craindre : 6 600 habitants au plus dans la Manche.

**9. La mer « hors commune » est fausse dans la baie du Mont-Saint-Michel.** Les herbus et les estrans ont un relief dans le RGE ALTI, mais ils ne sont dans aucune commune de la BD TOPO : la chaîne les met en mer. De même, Jersey et Guernesey, terres étrangères, sont classées en mer au lieu d'être grisées (G-01). Il faut définir la mer par le trait de côte (couche `limite_terre_mer`, « Plus hautes eaux », 692 km dans le fichier de la Manche) et par l'absence de relief, pas par les communes.

**10. Les données ne donnent pas la charge des ponts.** Une restriction de poids sur 1 pont sur 3 292. B-03 devra reposer presque entièrement sur une heuristique : importance et largeur de la route portée (connues pour plus de la moitié), longueur, et ce que le pont franchit.

**11. La végétation se décrit par surfaces, sans essences fines.** La BD TOPO distingue feuillus, conifères et mixte, plus bois, haies (233 000 surfaces et 902 000 lignes dans la Manche), landes, vergers et peupleraies ; les essences fines sont dans la BD Forêt, non téléchargée (B-04). Générer les arbres au chargement coûte 0,5 ms par tile.

### Limites

- **Un seul département**, plus Paris sans relief. Les départements de montagne (relief plus coûteux à compresser), la Guyane (forêt) et les grandes métropoles de province ne sont pas mesurés ; l'extrapolation de l'outre-mer reprend la répartition de la Manche.
- **Extrapolation** : surfaces terrestres et longueurs de côte nationales à vérifier ; les tiles de mer sont estimées à partir de la Manche.
- **Mer** : définie par les communes (voir l'analyse, point 9) ; ni bathymétrie (SHOM) ni marée.
- **Page cache chaud** pour le chargement : vider le cache demande les droits root. Un SSD NVMe lit 60 ko en moins de 0,1 ms ; un disque dur ajouterait un déplacement de tête par paquet.
- **Bâti de 1 m recalculé** à partir des tiles du département seulement : au bord du département, les voisines d'un autre département manquent (écarts du franchissable recalculé en bordure).
- **Ponts** : ce qui est franchi est une heuristique géométrique ; « route » est surestimé.
- **Lieux** : sélection par nature et toponyme, sans vérification une à une ; longueurs de piste et tirants d'eau non calculés.
- **Mairies de Paris, Lyon et Marseille** : rattachement par code faux pour les arrondissements.
- **Déterminisme** : Linux seulement ; projections par la bibliothèque mathématique du système.
- **Population** : l'écart de 1,5 à 3,4 % avec la population légale n'est pas expliqué en détail.

## Conclusion

- **B-01 : traitement avant distribution.** Le joueur télécharge 35 à 41 Go de tiles (France entière, hors Pacifique, relief de 2 m) au lieu de 594 Go de sources, et n'a rien à calculer. La chaîne tourne en 1,9 h sur la machine de référence, avec 11 Go de mémoire par département.
- **Une tile** : en-tête (part de bâti, habitants) + neuf sections zstd + empreinte, rangées par paquets de 16 × 16 tiles. 60 ko en rural, 120 ko en ville, 210 ko en centre dense ; les tiles de mer ne pèsent rien.
- **Relief : nœuds de 2 m, quantifiés à 10 cm**, bords compris : 48 ko par tile, 3 cm d'erreur RMS, aucune marche aux raccords.
- **Précalculés** : zone accessible, cours et passages, population par fragment, mairies, voisinage des communes, franchissable. **Recalculés au chargement** : bâti de 1 m, capacités, arbres, terrain des flow fields. Chargement d'une tile : 3 à 8 ms sur un thread, à étaler.
- **Déterminisme** : mêmes octets de 1 à 16 threads ; raccords exacts (0 marche, 0 doublon, population conservée).
- Constats pour la suite : population parisienne deux fois plus dense que l'hypothèse de P-01 ; charge des ponts quasi absente des données ; mer à redéfinir par le trait de côte ; Pacifique hors des données de l'IGN.

## Décisions

Prises par l'utilisateur sur les recommandations de ce prototype, et reportées dans [OPEN-PROBLEMS.md](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-01--chaîne-de-traitement-des-données-ign) (A-01, A-10, A-19, B-01, B-03, B-04, C-01, C-04, G-01, G-02), [SYSTEMES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/systemes/SYSTEMES.md#monde), le [README](https://github.com/labatailledefrance/documentation/blob/main/README.md) du jeu, `CLAUDE.md`, les fiches [mairies](https://github.com/labatailledefrance/documentation/blob/main/game-elements/locations/mairies.md), [CNPE](https://github.com/labatailledefrance/documentation/blob/main/game-elements/locations/cnpe.md), [barrages](https://github.com/labatailledefrance/documentation/blob/main/game-elements/locations/barrages.md) et [SNLE](https://github.com/labatailledefrance/documentation/blob/main/game-elements/units/naval/snle-triomphant.md).

- **Traitement avant distribution** (B-01) : le projet lance la chaîne, en Rust, un processus par département, à chaque édition de la BD TOPO ; le joueur télécharge **toutes les tiles à l'installation** (35 à 41 Go).
- **Format** : paquets de 16 × 16 tiles avec index ; sections zstd **niveau 19** ; empreinte BLAKE3 par tile ; **aucun fichier** pour les tiles de pleine mer.
- **Relief** : nœuds de **2 m**, bords compris, quantifiés à **10 cm**.
- **Précalculés** : zone accessible, cours et passages, franchissable, habitants par fragment, mairies, voisinage des communes, part de bâti. **Recalculés au chargement** : bâti de 1 m, capacités, arbres, terrain des flow fields. Chargement **étalé** sur plusieurs ticks (C-04).
- **Population** (A-10, A-19) : carreaux de 200 m répartis sur les bâtiments résidentiels, **complétés commune par commune jusqu'à la population légale** ; fragments au prorata des habitants de leurs bâtiments ; zombies calculés à la création de la partie.
- **Seuils de P-04 maintenus** (A-01) : saturation de la migration à 40 000, capacité à 65 000.
- **Mer** : trait de côte des plus hautes eaux et absence de relief ; **estrans** = terre, recouverte par la marée ; **îles étrangères** proches grisées (G-01, G-02).
- **Ponts** (B-03) : heuristique, avec des valeurs réelles pour quelques grands ouvrages.
- **Mairies** de Paris, Lyon et Marseille rattachées à leur arrondissement par leur position ; **CNPE et barrages** listés à la main (identifiants BD TOPO).
- **Pas de Pacifique** dans le jeu (Nouvelle-Calédonie, Polynésie française, Wallis-et-Futuna) : l'IGN ne les couvre pas ; un jeu de données homogène vaut ce retrait.
- **SNLE** basés à Saint-Pierre-et-Miquelon, simple élément de contexte : ils sont invisibles en jeu, l'île n'est pas aménagée.
- **Mémoire de la chaîne** : 11 à 16 Go par département ne sont pas jugés excessifs.

## Questions ouvertes

Aucune pour l'instant : les questions de ce prototype sont toutes tranchées. Restent à faire dans la chaîne, pour appliquer les décisions : la mer par le trait de côte, le complément à la population légale, le rattachement géométrique des mairies d'arrondissement, les listes de CNPE et de barrages.
