# P-08 — Génération des bâtiments

Prototype jetable. Problèmes : [B-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-02--génération-des-bâtiments), [B-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-01--chaîne-de-traitement-des-données-ign), [B-05](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-05--destructions-et-modifications-du-terrain), [C-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-02--niveaux-de-vue), [C-04](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-04--streaming-sans-écran-de-chargement), [C-05](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-05--rendu-des-foules), [C-06](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#c-06--lisibilité-en-ville), [A-13](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-13--sauvegardes), [F-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation). Fiche du prototype : [prototypes/README.md](../README.md#p-08--génération-des-bâtiments). Premier prototype qui **affiche** le monde réel : il part des tiles produites par [P-07](../p07-chaine-manche/README.md) et du rendu de [P-06](../p06-origine-flottante/README.md).

## Hypothèse testée

Chaque bâtiment du jeu est généré à partir de son emprise et de ses attributs BD TOPO, de façon déterministe, sauf les monuments. P-08 doit établir :

1. le **modèle** d'un bâtiment : volume extrudé depuis l'emprise réelle (trous et multipolygones compris), posé sur le relief de 2 m ; hauteur et étages, avec une règle de repli quand la hauteur manque ; forme du toit ; façades et fenêtres ;
2. la **porte unique** (B-02) : où la placer, sur la zone accessible de P-07, face à la rue ;
3. les **porches** aux cases percées par P-07 ;
4. les **états** (intact, endommagé, effondré), les gravats et ce que la sauvegarde garde (A-13) ;
5. les **niveaux de détail** des trois vues (C-02), et un critère pour les monuments ;
6. **générer ou stocker** les maillages ;
7. l'**organisation du rendu** : maillage fusionné par tile, entité par bâtiment ou instanciation ;
8. le **déterminisme** : mêmes octets quels que soient les threads et l'ordre.

Analogie : un bâtiment est généré comme un fichier de configuration à partir d'un gabarit et d'un inventaire. L'inventaire (la BD TOPO) donne l'emprise et quelques attributs ; le gabarit (le code de génération) en tire un volume complet. Tant que le gabarit et l'inventaire sont versionnés, le résultat se régénère à l'identique : on ne sauvegarde que ce que la partie a changé, comme un diff sur une configuration générée.

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, **NVIDIA RTX 3060 12 Go** (pilote 595.99, Vulkan), SSD NVMe, Linux 7.0. Rust 1.99, **Bevy 0.20.1** complet (même configuration que P-06 : Wayland en `dlopen`, ni audio ni manettes), compilation portable (sans `target-cpu=native`), `lto = "thin"`. Crates vérifiées sur crates.io le 2026-11-17 : `earcutr` 0.5.0 (triangulation), `i_overlay` 9.1.0 (opérations booléennes entières ; la 9.2.0 n'avait que deux jours), `rusqlite`, `zstd`, `blake3`, `rayon`, `png` comme P-07. Mesures du 2026-11-17.

Les pièges de P-06 s'appliquent : la RTX 3050 sans écran est écartée par `WGPU_ADAPTER_NAME="RTX 3060"` ; tous les temps de frame et toutes les captures sont faits **hors écran** (caméra rendue dans une image, sans winit ni Wayland), après 6 s d'attente pour la compilation des shaders. La mémoire GPU est lue par `nvidia-smi` (mémoire utilisée de toute la carte, bureau compris : on compare à une exécution sans tile).

## Méthode

### Données

Aucun téléchargement : les tiles de P-07 de la **Manche** (`data/generated/p07-manche`, 14 076 tiles, 593 511 bâtiments possédés) et de **Paris** (`data/generated/p07-paris`, 105 tiles, 126 581 bâtiments, sans relief), et la BD TOPO des deux départements (`data/sources`). Le prototype lit les paquets de P-07 par `pread`, comme le jeu ([src/donnees.rs](src/donnees.rs)) : sections `bati`, `relief`, `franchissable`, `zone` (cases hors zone, cases percées) et `reseau` (routes au sol, pour la porte).

**Attributs absents des tiles de P-07**, lus dans la BD TOPO par identifiant (`cleabs`) : altitudes minimale et maximale du toit, altitude minimale du sol, matériaux des murs et de la toiture. Vérifiés dans les fichiers :

| Attribut (bâtiments en service) | Manche (679 401) | Paris (606 117, petite couronne comprise) |
|---|---|---|
| `hauteur` absente | 2,5 % | 12,4 % |
| `nombre_d_etages` absent | 54 % | 38 % |
| altitudes du toit (min. et max.) toutes deux présentes | 76 % | 31 % |
| matériaux des murs et du toit | 43 % | 54 % |
| `hauteur` − (alt. min. du toit − alt. min. du sol) | écart moyen 1 cm | écart moyen 1,5 cm |

La `hauteur` de la BD TOPO est donc la hauteur **à la gouttière** depuis le **sol le plus bas**, et `altitude_maximale_toit − altitude_minimale_toit` donne la **hauteur du toit** : l'indice le plus direct de sa forme. Les matériaux sont des codes des fichiers fonciers sur deux chiffres (premier chiffre : 1 pierre, 2 meulière, 3 béton, 4 brique, 5 aggloméré, 6 bois ; toiture : 1 tuiles, 2 ardoises, 3 zinc ou aluminium, 4 béton ; à vérifier).

### Le modèle d'un bâtiment ([src/batiment.rs](src/batiment.rs))

Un bâtiment = ses polygones (anneau extérieur, trous), ses attributs, et sa **graine** (son identifiant BD TOPO, haché). Rien d'autre.

| Élément | Règle |
|---|---|
| Sol | relief de 2 m interpolé aux sommets de l'emprise, sur la tile et ses voisines (contexte 3 × 3) |
| Soubassement | du sol le plus bas (moins 0,5 m enfoui) au rez-de-chaussée ; rez-de-chaussée au sol le plus haut, à **3 m au plus** au-dessus du plus bas (au-delà, la façade s'enfonce dans la pente) |
| Hauteur (gouttière) | `hauteur` ; sinon étages × 3 m ; sinon altitudes du toit et du sol ; sinon **médiane de la tile** pour le même usage et la même classe d'emprise (au moins 5 bâtiments) ; sinon médiane de l'usage dans la tile ; sinon une constante par usage |
| Niveaux | `nombre_d_etages`, sinon hauteur / 3 m |
| Forme du toit | hauteur du toit connue : plat sous 0,5 m ; sinon deux pans (églises, industriel, plus de 1 000 m²), **mansarde** (logement de 10 m et plus, 3 niveaux et plus, toit de 2 m et plus, 8 m de large et plus), quatre pans (emprise ramassée, ou un sur quatre par la graine), sinon deux pans. Hauteur du toit inconnue : règle sur la nature, l'usage, la hauteur et l'emprise, avec une pente de 40° |
| Toit, géométrie | fonction **affine par morceaux** sur une partition convexe du rectangle englobant orienté (calipers sur l'enveloppe convexe) : 1 cellule (plat), 2 (deux pans), 4 (quatre pans), 5 (mansarde, brisis à 70°) ; pente bornée à 60° |
| Façades | un quadrilatère par arête, du rez-de-chaussée à la fonction du toit (les pignons apparaissent seuls) ; coordonnées de texture en **travées** de 3 m (arrondies par arête) et en **niveaux** |
| Fenêtres | dans la texture (une cellule par travée et par niveau) : aucune géométrie, aucun stockage ; la position d'une fenêtre se recalcule (arête, travée, niveau) |
| Couleurs | matériaux s'ils sont connus, sinon teinte tirée de la graine ; une seule matière, couleurs par sommet |

**Toit.** L'emprise réelle (trous compris) est triangulée une seule fois par earcut, puis chaque triangle est **coupé le long des plis** du toit ; chaque morceau tombe dans une seule cellule, où le toit est plan. Les points de coupe sont calculés de façon **canonique** (extrémités du segment rangées avant le calcul) : le mur et le toit, qui parcourent la même arête dans des sens opposés, obtiennent les mêmes bits, donc aucune fissure. Une première version découpait l'emprise cellule par cellule (Sutherland-Hodgman) : sur les emprises concaves, les ponts dégénérés faisaient déborder le toit jusqu'à 53 % de l'aire.

**Porche** (cases percées de P-07 dont le centre est dans l'emprise) : le bas du bâtiment est l'emprise **moins** les cases percées, jusqu'à 4 m (opération booléenne en entiers, `i_overlay`) ; le haut est l'emprise entière ; un plafond ferme le passage.

**Porte** ([src/porte.rs](src/porte.rs)) : distance à la rue de chaque case accessible (franchissable, hors des cases exclues de P-07) par un parcours en largeur sur une fenêtre de 1,4 km autour de la tile, depuis les routes au sol (natures « type autoroutier » à « chemin ») ; candidats tous les 0,5 m le long des arêtes extérieures d'au moins 1,6 m, devant une case accessible à 1, puis 2, puis 3 m du mur ; on garde la plus proche de la rue (égalités : éloignement, arête, abscisse). Sans candidat : règle de P-05 (case accessible la plus proche du centroïde), signalée. La porte est un panneau de 1,2 × 2,2 m.

**États** : *intact* ; *endommagé* : même volume, couleurs assombries (la barre de vie est un nombre, B-02) ; *effondré* : le volume disparaît, remplacé par des **gravats** sur les cases de 2 m de l'emprise dilatées de 0,3 × la hauteur (6 m au plus), chacune un bloc de hauteur tirée de la graine ; les mêmes cases deviennent infranchissables pour les humains et les véhicules (B-02, B-05).

**Niveaux de détail** (C-02) :

| Vue | Contenu |
|---|---|
| Isométrique | tout : soubassement, façades à fenêtres, toit, porches, porte |
| Lointaine | emprise simplifiée (Douglas-Peucker à 1 m, en entiers), murs d'un seul tenant, toit conservé ; ni fenêtres, ni porches, ni porte |
| Simplifiée (carte) | emprise plate simplifiée (ou rien : voir l'analyse) |

### Génération d'une tile ([src/generation.rs](src/generation.rs))

Une tile génère les bâtiments qu'elle **possède** (règle de P-07 : le centroïde), en entier, même s'ils débordent. Préparation : médianes de repli, modèles, grille d'accès et portes, cases percées par bâtiment. Puis chaque bâtiment, indépendamment, dans n'importe quel ordre et sur n'importe quel thread ; la fusion range les maillages par identifiant. Variante **étalée** : au plus 3 ms par tick (P-04), le bâtiment en cours finit toujours.

### Rendu ([src/rendu.rs](src/rendu.rs))

Une seule matière (`StandardMaterial`, texture de fenêtres répétée, couleurs par sommet), ombres en cascade, MSAA ×4, 1920 × 1080. Chaque tile est ancrée sur son centre en Lambert-93 (f64) ; son `Transform` est relatif à une origine recalée au-delà de **5 km** (C-01, P-06) ; les sommets sont en f32 locaux à la tile. Le sol est le relief décimé à 8 m (un plan à Paris). Organisations comparées :

| Organisation | Entités par tile | Mise à jour d'un bâtiment |
|---|---|---|
| `tuile` | 1 maillage fusionné | refusionner la tile |
| `morceaux` | 1 par carré de 250 m (16 au plus) | refusionner le morceau |
| `batiment` | 1 par bâtiment | remplacer le maillage du bâtiment |

**Streaming** (`survol`) : la caméra suit un trajet à vitesse constante ; les tiles à moins d'un rayon du point visé sont générées en **tâche de fond** (`AsyncComputeTaskPool`, 4 tâches au plus ; maillages de Bevy construits dans la tâche), puis envoyées au GPU par une **file** limitée en octets par frame ; déchargées au-delà de 1,3 rayon. Témoin : les mêmes tiles générées avant le vol.

### Déterminisme

- Tout part des sommets **entiers** de P-07 (1/16 m) ; enveloppe convexe, orientation et simplification sont en entiers ; les opérations booléennes aussi (`i_overlay`, sans table de hachage ni parallélisme).
- Le reste n'utilise que `+ − × ÷` et `sqrt`, arrondis exactement par IEEE 754 : **aucune** fonction de la bibliothèque mathématique du système (`sin`, `atan2`, `powf`, `hypot`) dans la génération. Rust ne fusionne pas `a × b + c` en FMA de lui-même.
- Chaque bâtiment ne dépend que de lui-même, de sa graine et de données fixes de la tile et de ses voisines ; la fusion est rangée par identifiant.
- Contrôle : empreinte BLAKE3 des octets du maillage fusionné (positions, normales, coordonnées de texture, couleurs, indices) de chaque tile, sur 1, 4 et 16 threads, dans l'ordre direct, inverse et mélangé, et sur deux exécutions.

### Scénarios ([run.sh](run.sh))

| Scénario | Mesure |
|---|---|
| 1. Toutes les tiles bâties (Manche, Paris) | temps par bâtiment et par tile, triangles et octets par niveau de détail, stockage compressé, portes, porches, contrôles ([resultats/](resultats)) |
| 2. Emprises pathologiques | très petites (< 10 m²), très grandes (> 5 000 m²), concaves (aire < 60 % de l'enveloppe), à cours, multipolygones, à cheval sur deux tiles, sans hauteur, plus de 200 sommets |
| 3. Débit | 1, 8 et 16 threads (en parallèle sur 400 tiles, et une tile seule en parallèle sur ses bâtiments) ; génération étalée à 3 ms par tick |
| 4. Streaming | survol de Paris (250 m d'altitude à 50 m/s ; 3 km à 400 m/s) et de la Manche (300 m à 200 m/s ; 5 km à 1 km/s, Cherbourg → Saint-Lô), avec et sans génération en cours |
| 5. Rendu | 1, 9 et 49 tiles parisiennes, trois organisations, deux répétitions ; maillages compressés, sans ombres, vue lointaine |
| 6. Destruction | un bâtiment endommagé puis effondré au centre de Paris et de Cherbourg : coût CPU, grille franchissable, rendu |
| 7. Déterminisme | deux exécutions, 1, 4 et 16 threads, trois ordres |
| 8. Repli de la hauteur | validation croisée sur les bâtiments dont la hauteur est connue |
| 9. Format | taille de la section `bati` avec les attributs proposés |

**Types de tiles** : ceux de P-07 (classement par habitants INSEE) : `vide`, `rural`, `bourg`, `urbain`, `dense`.

### Paramètres de prototype

Ce sont des **hypothèses de mesure**, pas des statistiques de jeu : niveau de 3 m ; travée de 3 m ; soubassement enfoui de 0,5 m et haut de 3 m au plus ; porche de 4 m ; toit plat sous 0,5 m ; pente par défaut de 40°, au plus 60°, brisis de mansarde à 70° ; simplification de 1 m en vue lointaine ; gravats sur 0,3 × la hauteur autour de l'emprise (6 m au plus) ; morceaux de 250 m ; envoi au GPU de 2 Mo par frame ; budget de 3 ms par tick ; seuil de recalage de 5 km.

Reproduire : `./run.sh` (41 min) ; `ESSAI=1 ./run.sh` valide toutes les commandes en 14 min (quelques tiles, vols courts). Captures seules : `./run.sh captures`. Tests : `cargo test --release` (5 tests) ; micro-bancs : `cargo test --release banc -- --ignored --nocapture`. Extrapolation : [outils/extrapoler.py](outils/extrapoler.py), lancé par le script. Résultats : [resultats/](resultats) ; images : [images/](images) ; journal : [resultats/run.log](resultats/run.log).

## Budgets

- **Simulation** : 3 ms par tick pour tout chantier d'une tile (P-04) ; un calcul plus long est étalé ou mis en tâche de fond (F-02).
- **Rendu** : 16,6 ms par frame à 60 images par seconde ; environ 100 000 entités rendues au plus (C-05, P-06).
- **Streaming** : une ou deux tiles par seconde à basse altitude, jusqu'à 16 tiles par seconde en vue lointaine à 1 km/s (P-07, C-04).
- **Distribution** : 35 à 41 Go de tiles pour la France (P-07, B-01).

## Mesures

### 1. Toutes les tiles bâties ([resultats/tiles-manche.csv](resultats/tiles-manche.csv), [resultats/tiles-paris.csv](resultats/tiles-paris.csv), [resultats/controles-manche.txt](resultats/controles-manche.txt), [resultats/controles-paris.txt](resultats/controles-paris.txt))

**720 092 bâtiments** générés : 593 511 dans les 6 095 tiles bâties de la Manche (2 min 16 s, contrôles compris), 126 581 dans les 105 tiles de Paris (29 s).

Par tile, sur un thread, médianes (maximum entre parenthèses) :

| Type | Tiles | Bâtiments | Génération | Grille d'accès + portes | Triangles (isométrique / lointaine / carte) | GPU, isométrique (brut / compressé) | Stocké (zstd 19¹) | Section `bati` de P-07 |
|---|---|---|---|---|---|---|---|---|
| vide | 194 | 4 | 0,04 ms | 5,3 + 0,01 ms | 222 / 126 / 12 | 19 / 8 ko | 1,5 ko | 0,1 ko |
| rural | 4 884 | 54 | 0,62 ms (7) | 5,8 + 0,1 ms | 3 800 / 2 362 / 160 | 338 / 144 ko | 23 ko | 1,2 ko |
| bourg | 892 | 194 | 2,6 ms (24) | 6,4 + 0,3 ms | 16 275 / 9 930 / 612 | 1,4 / 0,6 Mo | 93 ko | 4,2 ko |
| urbain | 58 | 1 042 | 16 ms (40) | 7,7 + 1,8 ms | 100 300 / 60 314 / 3 654 | 8,4 / 4,2 Mo | 515 ko | 24 ko |
| dense (Manche) | 2 | 2 572 | 45 ms (70) | 8,7 + 4,4 ms | 274 166 / 156 855 / 8 096 | 23 / 11 Mo | 1,4 Mo | 59 ko |
| dense (Paris) | 91 | 1 341 | 38 ms (80) | 7,9 + 4,0 ms | 209 249 / 105 092 / 7 769 | 18 / 8,8 Mo | 1,0 Mo | 47 ko |

¹ Maillage de la vue isométrique en encodage compact ([src/stockage.rs](src/stockage.rs)), compressé en zstd 9 sur toutes les tiles et ramené au niveau 19 par le rapport mesuré sur une tile sur dix (0,935).

Par bâtiment, en moyenne :

| | Manche | Paris |
|---|---|---|
| Temps de génération (médiane / p99 / max) | 9,1 / 41 / 639 µs | 16 / 121 / 1 334 µs |
| Triangles (isométrique / lointaine / carte) | 82 / 50 / 3,1 | 155 / 78 / 5,8 |
| Octets sur le GPU (format brut de Bevy, 48 octets par sommet) | 7,0 ko | 13,2 ko |
| Stocké (encodage compact, zstd 9) | 488 octets | 813 octets |
| Section `bati` de P-07 (emprise et attributs) | 22 octets | 36 octets |
| Fenêtres (travées × niveaux des murs extérieurs) | 23 | 136 |

Le bâtiment le plus lent de Paris (1,3 ms) fait 25 400 m², 685 sommets et 213 cases de porche. Le maximum de la Manche (639 µs, un bâtiment de 100 m² à 4 sommets, dans une tile de 15 µs en médiane) est une mesure isolée sous le seuil de remesure (1 ms) : du bruit.

**Contrôles** :

| Contrôle | Manche | Paris |
|---|---|---|
| Bâtiments générés / identifiants distincts / annoncés par les en-têtes | 593 511 / 593 511 / 593 511 | 126 581 / 126 581 / 126 581 |
| Échecs de triangulation du toit | 0 | 0 |
| Cases de 1 m de l'emprise (règle du centre, comme le bâti de P-07) | 76 940 162 | 34 476 836 |
| … sans volume au-dessus | 5 725 (7 × 10⁻⁵) | 1 813 (5 × 10⁻⁵) |
| Cases sous le toit hors de l'emprise | 210 (3 × 10⁻⁶) | 1 770 (5 × 10⁻⁵) |
| Aire du toit ≠ aire de l'emprise (écart > 0,1 %) | 8 | 41 |
| Dénivelé du sol sous l'emprise > 3 m / > 10 m | 9 752 / 164 | — (sans relief) |

Les écarts de cases sont des cas limites de la règle du centre (centre de case exactement sur une arête de triangle). Les écarts d'aire sont des **emprises auto-sécantes** de la BD TOPO (anneau en « nœud papillon ») : l'aire signée y compense deux lobes, alors que le toit couvre bien la surface dessinée.

### 2. Emprises pathologiques ([resultats/pathologiques-manche.csv](resultats/pathologiques-manche.csv), [resultats/pathologiques-paris.csv](resultats/pathologiques-paris.csv))

| Catégorie | Bâtiments (Manche / Paris) | Temps médian / p99 (µs) | Triangles médian / max | Échecs | Bâtiments avec écart raster |
|---|---|---|---|---|---|
| très petites (< 10 m²) | 31 784 / 9 826 | 4,5 / 25 | 32 / 540 | 0 | 0,3 % |
| très grandes (> 5 000 m²) | 252 / 237 | 18–55 / 190–640 | 130–348 / 5 939 | 0 | 0,4–1,6 % |
| concaves (aire < 60 % de l'enveloppe) | 2 909 / 2 635 | 20–33 / 93–242 | 142–216 / 4 787 | 0 | 2 % |
| à cours | 280 / 8 731 | 57 / 195 | 279–299 / 5 939 | 0 | 2–3 % |
| multipolygones | 0 / 0 | — | — | — | — |
| à cheval sur deux tiles | 15 640 / 5 249 | 10–28 / 49–171 | 63–156 / 1 123 | 0 | 1–2 % |
| sans hauteur | 15 700 / 10 032 | 4,6–5,0 / 30–64 | 32 / 1 376 | 0 | 0,4 % |
| plus de 200 sommets | — / 48 | 254 / 1 334 | 1 722 / 5 939 | 0 | 2 % |

Aucun bâtiment des tiles de P-07 n'est un multipolygone : la BD TOPO découpe chaque partie en un bâtiment. Les bâtiments à cheval sont générés une seule fois, par la tile qui les possède, avec le sol, les cases percées et la grille d'accès des voisines (contexte 3 × 3).

### 3. Modèle : hauteur, toit, porches ([resultats/modele-manche.csv](resultats/modele-manche.csv), [resultats/modele-paris.csv](resultats/modele-paris.csv), [resultats/repli-manche.csv](resultats/repli-manche.csv), [resultats/repli-paris.csv](resultats/repli-paris.csv))

| | Manche | Paris |
|---|---|---|
| Hauteur : donnée / étages / altitudes du toit / médiane de la tile / constante | 97,3 / 0,25 / 0,01 / 2,35 / 0,12 % | 92,0 / 1,2 / 0,00 / 6,8 / 0,03 % |
| Hauteur du toit connue (BD TOPO) | 76 % | 15 % |
| Toits plats / deux pans / quatre pans / mansardes | 12,9 / 45,4 / 41,4 / 0,4 % | 35,4 / 23,6 / 9,9 / 31,1 % |
| Bâtiments avec porche ; cases percées couvertes par un porche | 1 599 ; 96 % | 33 783 ; 99,7 % |

**Repli de la hauteur**, validé sur les bâtiments dont la hauteur est connue (erreur absolue) :

| Règle | Manche : médiane / p90 / biais | Paris : médiane / p90 / biais |
|---|---|---|
| constante de 6 m | 1,9 / 3,7 / +1,4 m | 11,3 / 18,3 / −10,4 m |
| constante par usage | 1,6 / 3,5 / +0,8 m | 10,6 / 17,6 / −9,8 m |
| médiane de la tile (usage × classe d'emprise) | 0,8 / 2,4 / −0,1 m | 3,2 / 9,9 / 0,0 m |
| étages × 3 m (quand ils sont connus : 48 %, 73 %) | 1,3 / 3,0 / −0,2 m | 2,4 / 9,5 / −0,9 m |
| **règle de P-08** (étages, sinon médiane, sinon constante) | **0,2 / 2,4 / −0,1 m** | **1,6 / 8,3 / −0,7 m** |

Dans les tiles de P-07, 8 % des bâtiments de Paris n'ont pas de hauteur (12 % dans le fichier départemental, petite couronne comprise ; surtout des logements et des annexes). Dans la tile de l'usine de La Hague (`x347_y6964`), 151 bâtiments sur 154 n'ont pas de hauteur et prennent la constante de leur usage (donnée retenue pour un site sensible ? à vérifier).

### 4. Portes ([resultats/portes-manche.csv](resultats/portes-manche.csv), [resultats/portes-paris.csv](resultats/portes-paris.csv), [images/](images))

| | Manche | Paris |
|---|---|---|
| Porte devant une case accessible à 1 m / 2 m / 3 m du mur | 93,4 / 4,4 / 1,8 % | 86,9 / 7,5 / 3,2 % |
| **Porte sur la zone accessible** (total) | **99,6 %** | **97,6 %** |
| Sans case au pied des murs → règle de P-05 | 0,4 % | 2,4 % |
| Sans aucune solution | 0 | 0 |
| Distance à la rue à pied, P-08 (médiane / p90) | 12 / 46 m | 6 / 42 m |
| Distance à la rue à pied, règle de P-05 (médiane / p90) | 26 / 60 m | 24 / 80 m |
| Règle de P-05 : case à plus de 3 m du bâtiment | 0,3 % | 6,6 % |
| Écart entre les deux portes (médiane / p90) | 8 / 14 m | 11 / 21 m |
| P-08 plus près de la rue / égal / plus loin que P-05 | 96,2 / 3,7 / 0,1 % | 92,7 / 6,5 / 0,5 % |
| Coût, par tile : grille d'accès (parcours en largeur, 700 × 700 cases) + portes | 5 à 9 ms + 0,01 à 7 ms | 6 à 9 ms + 0,3 à 8 ms |

La règle de P-05 place la porte au milieu de la cour ou de l'autre côté du bloc dès que le centroïde est loin de la rue ; à Paris, une sur quinze n'est même pas au pied du bâtiment. Sur les images ([portes-x651_y6862.png](images/portes-x651_y6862.png) et les autres tiles typiques), les portes de P-08 (rouge) sont sur la façade côté rue, celles de P-05 (bleu) souvent dans les cours.

### 5. Débit ([resultats/debit-manche.csv](resultats/debit-manche.csv), [resultats/debit-paris.csv](resultats/debit-paris.csv), [resultats/etale-manche.csv](resultats/etale-manche.csv), [resultats/etale-paris.csv](resultats/etale-paris.csv), [resultats/banc.txt](resultats/banc.txt))

Bâtiments par seconde (génération et fusion, contextes préparés), deux répétitions :

| Threads | Manche (400 tiles, 33 123 bâtiments) | Paris (105 tiles, 126 581 bâtiments) |
|---|---|---|
| 1 | 83 000 – 87 000 | 34 000 |
| 8 | 362 000 – 412 000 | 214 000 – 218 000 |
| 16 | 431 000 – 453 000 (× 5,2) | 247 000 – 286 000 (× 7,3 à 8,4) |

Une **tile seule** parallélisée sur ses bâtiments (Paris, 2 524 bâtiments) : 88 ms sur un thread, **12 à 29 ms** sur 8 ou 16 threads selon l'exécution (mesure répétée quatre fois, bimodale) : quelques gros bâtiments et l'ordonnancement dominent.

**Génération étalée** à 3 ms par tick ([resultats/etale-paris.csv](resultats/etale-paris.csv)) : tick médian 3,01 ms, **maximum 3,08 ms** ; centre de Paris 10 ticks (contre 45 ms d'un seul tenant, portes comprises), centre de Cherbourg 11 ticks (48 ms) ; avec la grille d'accès, les portes et la fusion, 17 et 18 ticks. Une tile rurale tient en un tick.

**Micro-banc**, par bâtiment, un thread ([resultats/banc.txt](resultats/banc.txt)) :

| Poste | Paris | Cherbourg |
|---|---|---|
| Complet (vue isométrique) | 28,6 µs | 16,8 µs |
| Sans porche | 21,6 µs | 15,9 µs |
| Sans porche, toits plats forcés | 8,0 µs | 4,4 µs |
| Vue lointaine | 15,9 µs | 12,1 µs |
| Carte | 3,9 µs | 2,4 µs |
| Triangulation de l'emprise seule | 2,4 µs | 1,0 µs |
| Modèle seul (hauteur, toit, couleurs) | 0,7 µs | 0,45 µs |
| Porte (hors grille) | 3,4 µs | 2,0 µs |

Les **toits à pans** coûtent les deux tiers : coupe des triangles le long des plis et dédoublonnage des sommets (table triée). Les porches ajoutent 7 µs en moyenne à Paris (opérations booléennes sur 27 % des bâtiments).

### 6. Générer ou stocker ([resultats/extrapolation.csv](resultats/extrapolation.csv), [resultats/extrapolation.txt](resultats/extrapolation.txt))

Moyennes par type de tile, rapportées aux 633 436 tiles bâties de la France (comptes de P-07) :

| Type | Tiles | Section `bati` de P-07 | Maillages stockés (isométrique) | … et vue lointaine | Génération (un thread) |
|---|---|---|---|---|---|
| vide | 175 540 | 247 o | 3,4 ko | 5,5 ko | 0,1 ms |
| rural | 363 734 | 1,3 ko | 25 ko | 41 ko | 0,7 ms |
| bourg | 81 297 | 5,7 ko | 124 ko | 200 ko | 3,5 ms |
| urbain | 11 227 | 27 ko | 592 ko | 947 ko | 18 ms |
| dense | 1 638 | 49 ko | 1,05 Mo | 1,6 Mo | 39 ms |
| **France** | 633 436 | **1,4 Go** | **28 Go** | **45 Go** | **0,2 h** |

Stocker les maillages multiplierait la section des bâtiments par 20 et presque doublerait les 35 à 41 Go de P-07 (73 à 86 Go avec la vue lointaine). Les générer prend 0,2 h pour toute la France sur **un** thread ; au chargement, 0,6 à 80 ms par tile, en tâche de fond.

### 7. Rendu : 1, 9 et 49 tiles parisiennes ([resultats/cadence.csv](resultats/cadence.csv))

Temps de frame hors écran (médiane / p99), 600 frames, ombres en cascade, orbite lente ; mémoire GPU de la carte moins celle d'une exécution sans tile (1 906 Mo).

| Tiles (triangles) | `tuile` | `morceaux` (250 m) | `batiment` | `morceaux`, compressés | `morceaux`, sans ombres | `morceaux`, vue lointaine |
|---|---|---|---|---|---|---|
| 1 (0,17 M) | 1,47 / 1,8 ms ; +4 Mo | 1,47 / 1,8 ms | 1,5 / 1,9 ms | 1,48 / 1,8 ms | 1,27 / 1,6 ms | 1,47 / 1,8 ms |
| 9 (2,5 M) | 1,8 / 3,2 ms ; +680 Mo | 1,6 / 3,0 ms ; +285 Mo | 1,75 / 2,5 ms ; +260 Mo | 1,44 / 2,1 ms ; +60 Mo | 1,46 / 2,7 ms | 2,5 / 3,0 ms |
| 49 (12,4 M) | 4,6 / 4,9 ms ; +1,96 Go | 4,05 / 4,4 ms ; +1,99 Go | 3,5 / **7,3 ms, max 48 ms** ; **+3,4 à 3,7 Go** | **2,9 / 3,2 ms ; +1,17 Go** | 4,0 / 4,3 ms | 4,7 / 5,1 ms ; +1,15 Go |
| Entités (49 tiles) | 98 | 832 | 76 158 | 832 | 832 | 832 |

Les maillages des 49 tiles font 1,04 Go au format brut de Bevy (48 octets par sommet) et 0,52 Go au format **compressé** de Bevy 0.20 (`MeshCompressionArgs::regular` : positions et normales sur 16 bits, couleurs sur 8 bits ; 20 octets par sommet), sans changer de shader. La vue lointaine est vue de plus loin (plus de tiles dans le champ) : son temps de frame n'est pas comparable aux autres colonnes.

### 8. Streaming ([resultats/survol.csv](resultats/survol.csv), un fichier par vol)

Organisation `morceaux`, envoi au GPU limité à 2 Mo par frame ; génération en tâche de fond (« gén. ») ou avant le vol (« témoin ») :

| Vol | Tiles générées | Génération d'une tile (méd. / max) | Frame méd. / p99 / max, gén. | idem, témoin | Frames > 16,6 ms (gén. / témoin) | Recalages (frame max) | Tiles proches manquantes |
|---|---|---|---|---|---|---|---|
| Paris, 250 m, 50 m/s, rayon 1,5 km | 39 | 58 / 104 ms | 1,75 / 2,75 / 13,4 ms | 1,72 / 2,75 / 11,2 ms | 0 / 0 | 2 (2,1 ms) | 0 |
| Paris, 250 m, **200 m/s** | 39 | 56 / 108 ms | 1,68 / 3,53 / 12,9 ms | — | 0 | 2 (1,8 ms) | 0 |
| Paris, 3 km, 400 m/s, vue lointaine, rayon 6 km | 105 | 26 / 68 ms | 3,9 / 6,9 / 25,3 ms | 4,0 / 7,0 / 25,0 ms | 3 / 2 | 1 (2,5 ms) | 0 |
| Manche, 300 m, 200 m/s | 63 | 7,5 / 158 ms | 1,49 / 2,01 / 5,9 ms | 1,48 / 1,88 / 5,1 ms | 0 / 0 | 3 (1,6 ms) | 0 |
| Manche, 5 km, **1 km/s**, vue lointaine, rayon 8 km (Cherbourg → Saint-Lô) | 1 306 | 3,6 / 110 ms | 5,1 / 10,4 / 23,7 ms | 5,1 / 10,5 / 25,1 ms | 1 / 1 | 14 (5,7 ms) | 0 |

Les rares frames au-dessus du budget sont présentes avec et sans génération : elles viennent de l'envoi d'un gros morceau (au moins un morceau par frame, même au-delà de 2 Mo), pas du calcul. Les frames où une génération tourne ont un p99 un peu plus haut (4,2 contre 2,7 ms à Paris) : les tâches de fond prennent des cœurs.

**Témoins de l'organisation** (Paris, 250 m, 50 m/s) :

| Organisation, envoi | Frame méd. / p99 / max | Frames > 16,6 ms |
|---|---|---|
| `tuile`, d'un bloc | 2,02 / 3,27 / 30,5 ms | 20 |
| `tuile`, file de 2 Mo (une tile = un asset de 15 Mo, indivisible) | 2,03 / 3,40 / 30,6 ms | 21 |
| `morceaux`, file de 2 Mo | 1,75 / 2,75 / 13,4 ms | **0** |
| `batiment`, file de 2 Mo | 1,74 / 2,84 / 11,4 ms | 0 |

Les à-coups suivent exactement l'arrivée d'une tile : Bevy recopie dans la même frame **tous** les maillages ajoutés (`allocate_and_free_meshes`), avant son propre limiteur d'octets (`RenderAssetBytesPerFrame`), qui ne découpe pas un asset. Agrandir les « dalles » de l'allocateur ne change rien (essai hors campagne, chaque maillage de plus de 1 Mio dans son propre tampon : 21 frames au-dessus du budget, comme sans réglage). Seul le découpage en assets plus petits, envoyés par une file, supprime les pics. Construire le maillage de Bevy dans la tâche de fond ramène le coût du thread principal à l'arrivée d'une tile de 14,5 ms à 0,01 ms.

### 9. Destruction ([resultats/destruction-paris.csv](resultats/destruction-paris.csv), [resultats/destruction-cherbourg.csv](resultats/destruction-cherbourg.csv), [resultats/rendu-destruction.csv](resultats/rendu-destruction.csv), [resultats/sauvegarde-paris.csv](resultats/sauvegarde-paris.csv), [images/](images))

Coûts CPU, au centre de Paris (bâtiment médian de 149 m², puis le plus grand près du centre, 12 450 m²) et de Cherbourg :

| Étape | Bâtiment régénéré | Refusion de la tile | Refusion du morceau | Triangles avant → après | Cases franchissables bloquées (mise à jour) |
|---|---|---|---|---|---|
| Endommagé, 149 m² | 3,8 µs | 0,9 ms | 0,09 ms | 35 → 35 | — |
| Effondré, 149 m² | 31 µs | 0,7 ms | 0,10 ms | 35 → 1 530 | 34 (0,4 µs) |
| Endommagé, 12 450 m² | 436 µs | 0,8 ms | 0,02 ms | 4 787 → 4 787 | — |
| Effondré, 12 450 m² | 1,0 ms | 1,5 ms | 0,22 ms | 4 787 → 53 710 | 2 296 (14 µs) |
| Effondré, hangar de 35 473 m² (Cherbourg) | 2,0 ms | 2,2 ms | 0,51 ms | 65 → **103 180** | 1 543 (17 µs) |

**Rendu** (bloc de 9 tiles, bâtiment de 12 450 m²) : mise à jour complète sur le thread principal (régénération, fusion, maillage de Bevy) et pire frame des six suivantes :

| Organisation | Mise à jour | Pire frame suivante |
|---|---|---|
| `tuile` | 28,8 ms | 30,9 ms |
| `morceaux` | 11,8 ms | 13,0 ms |
| `batiment` | 9,1 ms | 11,6 ms |

La conversion en maillage de Bevy et l'envoi des gravats (54 000 triangles en blocs de 2 m) dominent ; en jeu, elles iraient en tâche de fond comme le streaming.

**Sauvegarde** (A-13) : seuls les bâtiments modifiés, triés (identifiant en différence, état, barre de vie sur 16 bits pour les endommagés), zstd : 46 octets pour 1 % des bâtiments d'une tile dense, 300 octets pour 10 %, **1,4 à 2,4 ko** pour tous. Le volume, les gravats et les cases bloquées se régénèrent depuis l'état.

### 10. Déterminisme ([resultats/determinisme-paris-1.csv](resultats/determinisme-paris-1.csv), [resultats/determinisme-paris-2.csv](resultats/determinisme-paris-2.csv), [resultats/determinisme-manche-1.csv](resultats/determinisme-manche-1.csv), [resultats/determinisme-manche-2.csv](resultats/determinisme-manche-2.csv))

| Variante | Paris (105 tiles) | Manche (5 tiles typiques) |
|---|---|---|
| 1 thread, ordre direct | `2de7a257…` | `135a7e49…` |
| 16 threads, direct ; 16, inverse ; 16, mélangé ; 4, mélangé | identiques, 0 tile différente | identiques, 0 tile différente |
| Seconde exécution | identique | identique |

Les **octets des maillages** (positions, normales, coordonnées de texture, couleurs, indices) sont identiques. La seule fonction de la bibliothèque mathématique du système appelée par le prototype (`powf`, conversion des couleurs en linéaire) est dans le rendu, hors de l'empreinte.

### 11. Monuments ([resultats/monuments-manche.csv](resultats/monuments-manche.csv), [resultats/monuments-paris.csv](resultats/monuments-paris.csv))

| Nature BD TOPO | Manche (aire médiane) | Paris (aire médiane) |
|---|---|---|
| Église | 835 (314 m²) | 255 (531 m²) |
| Chapelle | 195 (77 m²) | 76 (309 m²) |
| Château | 410 (290 m²) | 5 (735 m²) |
| Tour, donjon | 238 (46 m²) | 17 (87 m²) |
| Fort, blockhaus, casemate | 508 (68 m²) | — |
| Moulin à vent | 11 (29 m²) | 4 (48 m²) |
| Monument | 3 (151 m²) | 199 (**3 m²**) |
| Arc de triomphe ; arène ou théâtre antique | — | 9 ; 2 |
| Phare | 0 dans les tiles | — |

La nature « Monument » recouvre à Paris des statues et des stèles (3 m² en médiane) ; l'abbaye du Mont-Saint-Michel est une « Église » de 3 577 m², parmi 834 autres dans la Manche. Les 508 blockhaus du mur de l'Atlantique sont une nature à part.

### 12. Captures ([images/](images))

Vue isométrique (`*-iso.png`) et vue lointaine (`*-loin.png`, 3 × 3 tiles) du centre de Paris, de Cherbourg, de Saint-Lô, du bocage, de La Hague et du Mont-Saint-Michel ; destruction au centre de Paris (`destruction-paris-intact.png`, `-endommage.png`, `-effondre.png`) ; portes en vue de dessus (`portes-*.png` : bâti en brun, distance à la rue en gris, routes en bleu, cases percées en orange, portes de P-08 en rouge, règle de P-05 en bleu). Toutes prises hors écran, après la compilation des shaders.

## Analyse

**1. Le modèle tient sur l'emprise réelle, sans cas d'échec.** 720 092 bâtiments, aucun échec de toit, aucune case de bâti de P-07 sans volume au-delà de 7 × 10⁻⁵, aucun volume hors emprise au-delà de 5 × 10⁻⁵, aucun doublon. La clé est de trianguler l'emprise **une seule fois** (earcut est fiable sur les vrais polygones), puis de couper les triangles le long des plis du toit : découper l'emprise d'abord, comme on l'aurait fait naturellement, fabrique des polygones dégénérés et des toits qui débordent de moitié. Le toit décrit comme une fonction affine par morceaux donne gratuitement les pignons, les croupes et les mansardes sur n'importe quelle emprise. Ses limites se voient sur les captures : les bâtiments **à cour** n'ont de pente que côté rue (Palais-Royal), les **L** ont un seul faîtage. Le remède connu est le squelette droit (*straight skeleton*), plus coûteux et fragile ; ce n'est pas un défaut de jeu, c'est une question de rendu à trancher.

**2. La hauteur du toit est le meilleur attribut absent des tiles.** `altitude_maximale_toit − altitude_minimale_toit` existe pour 76 % des bâtiments de la Manche, et dit directement plat ou à pans. Avec elle, la forme du toit est une donnée pour les trois quarts des bâtiments ruraux ; sans elle, une règle. À Paris, elle manque à 85 % : la règle (mansarde pour les immeubles de 10 m et plus) y fait l'essentiel. Les matériaux (la moitié des bâtiments) ne servent qu'aux couleurs. Les trois coûtent ensemble environ 1,5 octet par bâtiment.

**3. Le repli de la hauteur doit être local.** Une constante se trompe de 11 m à Paris ; la médiane de la tile pour le même usage et la même classe d'emprise, de 3 m ; les étages, quand ils existent, de 2,4 m. La règle de P-08 (étages, sinon médiane locale, sinon constante) tombe à 0,2 m dans la Manche et 1,6 m à Paris. Elle ne dépend que du contenu de la tile : même résultat quel que soit l'ordre de génération.

**4. La porte doit regarder la rue, et la règle de P-05 ne le fait pas.** Avec la case accessible au pied des murs la plus proche de la rue à pied, 97,6 à 99,6 % des portes sont au pied du bâtiment sur la zone accessible, deux fois plus près de la rue que par la règle de P-05, qui en met une sur quinze hors du bâtiment à Paris. Le reste (0,4 à 2,4 %) n'a aucune case accessible au pied des murs : bâtiments enclavés dans un îlot, que la règle de P-05 rattache à la case la plus proche ; aucun cas sans solution. Mais la règle demande la zone accessible et la grille franchissable **des tiles voisines** et un parcours en largeur de 5 à 9 ms par tile : c'est un calcul de chaîne, comme la zone elle-même (P-04, P-07). Stockée, une porte coûte 1,6 octet (rang de l'arête et abscisse). Analogie : on ne recalcule pas la table de routage à chaque démarrage d'un routeur, on la distribue.

**5. Générer, sans hésiter.** Stocker les maillages coûterait 28 Go (45 Go avec la vue lointaine) pour la France, vingt fois la section des bâtiments, presque autant que toutes les tiles de P-07. Générer coûte 0,6 ms par tile rurale et 16 à 80 ms par tile urbaine ou dense sur un thread, 0,2 h pour toute la France, et se fait en tâche de fond pendant le survol sans jamais manquer une tile proche, même à 1 km/s en vue lointaine (1 306 tiles en 69 s). Analogie de P-07 : on ne distribue pas les binaires de chaque configuration possible, on distribue la configuration et le gabarit.

**6. Le coût d'une tile se règle hors des ticks de simulation.** D'un seul tenant, une tile dense prend 38 à 80 ms : trop pour un tick. Étalée à 3 ms, elle tient en 10 à 18 ticks sans dépasser 3,08 ms ; en tâche de fond (le choix du streaming), elle ne touche pas aux ticks du tout. Paralléliser une tile sur ses bâtiments est décevant et instable (12 à 29 ms sur 16 threads) ; paralléliser sur les tiles monte à × 5 à × 8.

**7. Le GPU n'est pas la limite ; l'envoi l'est.** 49 tiles parisiennes (12,4 millions de triangles) s'affichent en 4 ms, 3 ms avec les maillages compressés de Bevy, qui divisent aussi la mémoire par deux sans changer de shader. Ce qui casse le budget, c'est d'**envoyer** 15 Mo d'un coup : Bevy recopie tous les nouveaux maillages dans la frame où ils arrivent, quel que soit son propre limiteur. Il faut donc des assets de 1 à 2 Mo et une file d'envoi : les **morceaux de 250 m** (16 par tile) le font, et ramènent les frames au-dessus du budget de 20 à 0. Analogie : un lien saturé par un gros transfert ; on découpe en paquets et on applique une file d'attente, pas un lien plus gros.

**8. Une entité par bâtiment est le mauvais compromis à l'échelle de la vue.** Elle tient en vue isométrique (1 à 9 tiles, 15 000 entités) et met à jour un bâtiment au plus vite (9 ms), mais à 49 tiles elle fait 76 000 entités, près de la limite de C-05, des pics à 48 ms et 75 % de mémoire GPU en plus ; chaque recalage de l'origine touche toutes ces entités (P-06). Les morceaux de 250 m sont le bon grain : 832 entités pour 49 tiles, un recalage gratuit (un `Transform` par morceau ; la frame du recalage prend 1,6 à 5,7 ms), une destruction qui ne refait que 0,02 à 0,5 ms de fusion. L'**instanciation** ne sert pas aux volumes, tous différents ; elle ne servirait qu'à des éléments répétés (fenêtres en relief, portes, gravats), aujourd'hui portés par la texture.

**9. La destruction est bon marché, sauf la géométrie des gravats.** Endommager ne change que les couleurs ; effondrer régénère un bâtiment en 0,03 à 2 ms et bloque ses cases en 14 µs. Mais un bloc par case de 2 m transforme un hangar de 35 000 m² en 103 000 triangles : il faut fusionner les blocs (un relief de gravats par bâtiment). La sauvegarde ne garde que l'état : 1,4 à 2,4 ko par tile même si tout est détruit, le reste se régénère (A-13).

**10. C-06 se fera dans le shader, pas dans l'organisation.** Rendre un bâtiment transparent ou le découper autour d'une unité, frame après frame, ne peut pas passer par une refusion (10 à 30 ms). Avec des morceaux fusionnés, il faut l'**indice du bâtiment par sommet** (4 octets) et une table d'états par morceau lue par le shader (transparence, découpe), ou une découpe purement géométrique (sphère ou cylindre autour de l'unité) : sujet de P-12. Une entité par bâtiment permettrait de changer de matière par bâtiment, au prix du point 8.

**11. Le déterminisme ne coûte rien, et ne dépend d'aucune bibliothèque mathématique.** Mêmes octets sur 1, 4 et 16 threads, dans trois ordres, sur deux exécutions. La génération n'utilise que des entiers, `+ − × ÷` et `sqrt`, comme la chaîne de P-07 pour les altitudes ; elle est donc portable sous Windows sans bibliothèque mathématique en Rust pur, contrairement aux projections de P-07 (P-18). Deux dépendances sont à figer comme zstd : earcut (l'ordre de ses triangles fait partie des octets) et i_overlay.

**12. Les monuments ne se repèrent pas par la nature.** « Monument » désigne des statues à Paris ; « Église » mêle 835 églises de village et l'abbaye du Mont-Saint-Michel. Le Mont montre aussi pourquoi le gabarit ne suffit pas : un bâtiment juché sur un rocher, dont la hauteur BD TOPO se compte depuis le pied de la pente, devient un bloc massif à façade enterrée (et, avant le plafond du soubassement, un mur aveugle de plusieurs dizaines de mètres). Un critère automatique ne trie pas les monuments à modéliser fidèlement ; une **liste écrite à la main** (identifiants BD TOPO), comme pour les CNPE et les barrages (P-07), le fait. Les natures servent à choisir un **gabarit** : église (nef et clocher), château, fort, phare, moulin, silo.

**13. Les emprises de la BD TOPO ont des défauts que la chaîne doit nettoyer.** 49 emprises auto-sécantes sur 720 000 faussent l'aire (donc la capacité de P-05 et la population de P-07) et l'aspect du toit. `i_overlay` les répare en entiers (union à remplissage non nul), à faire dans la chaîne.

### Limites

- **Deux jeux de données** : la Manche (rurale, une ville moyenne) et Paris (sans relief, dense) ; pas de montagne, de grande ville de province, ni d'outre-mer.
- **Toits** : rectangle englobant orienté, pas de squelette droit ; cours et ailes approximées. Pas de débord de toit, de cheminée, de lucarne.
- **Façades** : une seule texture de fenêtres, sans mipmaps (moiré à distance, sans objet en vue lointaine qui n'a pas de fenêtres) ; pas de vitrines au rez-de-chaussée, pas de variation par époque.
- **Porches** : passages de 4 m de haut, sans voûte ni portail.
- **Gravats** : blocs de 2 m, non fusionnés ; pas de projection sur les voisins ni de blessés (B-02).
- **Portes** : un panneau sombre ; la grille d'accès ignore les portes des autres bâtiments et les entrées de cour.
- **Rendu** : une matière, sans transparence ni découpe (C-06) ; mémoire GPU lue sur toute la carte (bureau compris, bruit de quelques dizaines de Mo) ; temps GPU non séparé du temps de frame (P-06).
- **Sol** : relief à 8 m pour l'image, sans routes ni végétation ; le sol en 2 m des flow fields n'est pas rendu.
- **Vues** : la carte (vue simplifiée) n'est mesurée qu'en triangles ; le passage d'une vue à l'autre (C-02, P-10) n'est pas testé.
- **Déterminisme** : Linux seulement.

## Conclusion

- **Modèle** : volume extrudé depuis l'emprise réelle, soubassement de 3 m au plus, hauteur de la BD TOPO avec un repli local (0,2 à 1,6 m d'erreur médiane), toit en fonction affine par morceaux (plat, deux pans, quatre pans, mansarde) choisi d'après la hauteur du toit de la BD TOPO, fenêtres dans la texture, porches par opération booléenne. Aucun échec sur 720 092 bâtiments.
- **Porte** : sur la case accessible au pied des murs la plus proche de la rue à pied ; 97,6 à 99,6 % des bâtiments, deux fois plus près de la rue que la règle de P-05.
- **Générer au chargement** : 0,6 à 80 ms par tile sur un thread, 0,2 h pour la France ; stocker coûterait 28 à 45 Go.
- **Rendu** : morceaux de 250 m fusionnés, envoyés par une file de 2 Mo par frame, maillages compressés : 49 tiles parisiennes en 3 ms, aucune frame au-dessus du budget à basse altitude, génération en tâche de fond sans tile manquante jusqu'à 1 km/s.
- **États** : endommagé = couleurs ; effondré = gravats et cases bloquées ; la sauvegarde ne garde que l'état (au plus 2,4 ko par tile).
- **Déterminisme** : mêmes octets quels que soient les threads et l'ordre, sans bibliothèque mathématique.

## Recommandations

- **Modèle et niveaux de détail** : celui de P-08 ; trois niveaux : isométrique (complet), lointaine (emprise simplifiée à 1 m, sans fenêtres, porches ni portes : −40 à −50 % de triangles), **pas de bâtiments en 3D dans la vue simplifiée** : le bâti de 1 m, déjà recalculé au chargement (P-07), sert de texture de carte.
- **Porte** : la règle de P-08, **calculée par la chaîne** (elle a la zone et les voisines) et stockée avec le bâtiment.
- **Générer au chargement**, en tâche de fond, tile par tile ; ne rien stocker ; garder l'étalement à 3 ms par tick pour les cas où la simulation a besoin des bâtiments sans rendu.
- **Rendu** : morceaux de 250 m fusionnés, file d'envoi limitée en octets par frame, format compressé de Bevy ; ni entité par bâtiment, ni instanciation des volumes ; indice du bâtiment par sommet pour C-06 (P-12).
- **Monuments** : une liste écrite à la main (identifiants BD TOPO) pour les monuments modélisés fidèlement ; des gabarits par nature pour les églises, châteaux, forts, phares et moulins.
- **Format de tile** (B-01) : ajouter à la section `bati` la **hauteur du toit** (dm), les **matériaux** des murs et du toit (un octet chacun) et la **porte** (rang de l'arête extérieure, abscisse au 1/16 m) : environ 3 octets par bâtiment, +8 à 16 % de la section, environ 0,2 Go pour la France ; **nettoyer les emprises auto-sécantes**.
- **Gravats** : fusionner les blocs d'un bâtiment effondré en un seul relief.

## Décisions

Prises par l'utilisateur sur les recommandations de ce prototype (toutes suivies), et reportées dans [OPEN-PROBLEMS.md](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-02--génération-des-bâtiments) (B-01, B-02, B-05, C-02, C-04, C-05, C-06, A-13), [SYSTEMES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/systemes/SYSTEMES.md#monde) et `CLAUDE.md`.

- **Modèle** d'un bâtiment : celui de P-08. Emprise réelle extrudée, posée sur le relief de 2 m avec un soubassement de hauteur bornée ; hauteur de la BD TOPO, sinon étages, sinon altitudes du toit, sinon médiane de la tile (même usage, même classe d'emprise), sinon constante par usage ; toit plat, à deux pans, à quatre pans ou en mansarde, choisi d'après la hauteur du toit, sur le **rectangle englobant orienté** (pas de squelette droit) ; fenêtres dans la texture, jamais stockées ; porches par opération booléenne aux cases percées.
- **Niveaux de détail** : vue isométrique complète ; vue lointaine (emprise simplifiée, sans fenêtres, porches ni portes) ; **aucun bâtiment en 3D en vue simplifiée** (bâti de 1 m en texture de carte).
- **Porte** : case accessible au pied d'un mur extérieur, la plus proche de la rue à pied ; **calculée par la chaîne** et stockée avec le bâtiment. Sans case au pied des murs : règle de P-05 (point du mur le plus proche de la case accessible la plus proche du centroïde).
- **Générer au chargement**, en tâche de fond, sans rien stocker.
- **Rendu** : **morceaux de 250 m** fusionnés, envoyés au GPU par une file limitée en octets par frame, au format compressé de Bevy ; ni entité par bâtiment, ni instanciation des volumes ; **indice du bâtiment par sommet** pour la transparence et la découpe (C-06, P-12).
- **Monuments** : **liste écrite à la main** d'identifiants BD TOPO ; **gabarits par nature** pour les églises, chapelles, châteaux, forts, phares et moulins.
- **États** : endommagé = couleurs seulement ; effondré = un seul relief de gravats sur l'emprise dilatée de 0,3 × la hauteur (6 m au plus, valeurs de départ), cases infranchissables pour les humains et les véhicules.
- **Format de tile** (B-01) : la section `bati` reçoit la hauteur du toit, les matériaux des murs et du toit, et la porte ; la chaîne nettoie les emprises auto-sécantes.
- **Sauvegarde** (A-13) : identifiant, état et barre de vie des seuls bâtiments modifiés, dans le fichier de leur tile ; tout le reste se régénère.
- **Liste des monuments** : courte (quelques dizaines de sites nationaux), écrite à la main ; pas de téléchargement de la base Mérimée.
- **Gabarits par nature** : une forme simple par nature (église : nef et clocher ; château : corps et tours d'angle ; fort, phare, moulin : volume caractéristique), sans détail architectural.
- **Endommagé** : dès que la barre de vie est entamée (pas de seuil).
- **Indice du bâtiment par sommet** : sur 16 bits, rang dans le morceau (2 octets par sommet).
- Toutes ces décisions sont **révisables** : elles fixent un point de départ pour avancer sur les prototypes suivants.

## Questions ouvertes

Aucune : les questions soulevées par les décisions ont été tranchées selon les recommandations (voir *Décisions*). Toutes ces décisions restent révisables.

Travail à faire, sans décision attendue : étendre la chaîne de P-07 (hauteur du toit, matériaux, porte, nettoyage des emprises) ; fusionner les gravats en un relief ; écrire la liste des monuments ; prototype P-12 pour la découpe.
