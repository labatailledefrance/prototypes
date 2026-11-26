# P-04 — Activation et désactivation des tiles

Prototype jetable. Problèmes : [A-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-01--nombre-de-zombies--simulation-par-tiles), [A-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-02--accélération-du-temps), [A-03](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-03--unités-dispersées-sur-tout-le-territoire), [A-13](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-13--sauvegardes), [A-16](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-16--gestion-des-corps). Fiche du prototype : [prototypes/README.md](../README.md#p-04--activation-et-désactivation-des-tiles). Point de départ : [P-03](../p03-flow-fields/README.md).

## Hypothèse testée

P-01 à P-03 mesurent des tiles **fixes** : actives du début à la fin, remplies une fois au démarrage. Dans le jeu, l'ensemble des tiles actives change sans arrêt : une colonne qui avance active les tiles devant elle et en désactive derrière elle ; une patrouille fait des allers-retours en limite de tile ; une base abandonnée redevient inerte puis est reprise ; les zombies des tiles inertes migrent.

Peut-on transformer « X zombies » en agents complets, et inversement, **sans incohérence et sans saccade** ? Plus précisément :

- **conservation exacte** du nombre de zombies à chaque tick (activation, désactivation, restauration, passages de frontière, migration, tirs) ;
- **même graine, mêmes positions** ; résultat indépendant du nombre de threads et du découpage en lots ;
- **sauvegarde puis restauration identiques** (zombies, corps, constructions et leur état) ;
- **activation étalée** sur plusieurs ticks, sans pic visible à ×1 ni au budget de ×10 ;
- coût et taille de la sauvegarde ; fréquence des activations et des désactivations ;
- franchissement des frontières par les zombies, entre tiles actives et vers des tiles qui ne le sont pas.

Analogie : une tile active est un conteneur démarré, une tile inerte un conteneur arrêté dont l'état est sur disque. Le prototype mesure le démarrage à froid, l'arrêt propre, la migration à chaud entre nœuds, et vérifie qu'aucun processus ne se perd en route.

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, SSD NVMe, Linux 7.0. Rust 1.99, `bevy_ecs` 0.20.1. Mesures du 2026-11-15.

## Méthode

### Point de départ

Le code de P-03 est copié dans [src/](src) (P-01 à P-03 ne sont pas modifiés), avec sa configuration retenue : arbre k-d des humains par tile, relais filtré, veille, veille profonde au-delà de la vue, flow fields de 2 m étalés, grille dense des zombies en u16, réservation exacte partout. La vue est de 400 m.

Nouveauté : un **bloc de tiles réelles adjacentes**, et non plus des tiles indépendantes. Le bloc fait 11 × 5 = **55 tiles** (x 646 à 656, y 6859 à 6863 en km Lambert-93), d'ouest en est à travers Paris, à partir des tiles de `data/generated/p01-tiles/`. La carte (bâti à 1 m, zone accessible, constructions) est résidente en mémoire pour tout le bloc ([src/carte.rs](src/carte.rs)).

**Densité** : sans les données INSEE carroyées, le nombre de zombies d'une tile est tiré de sa part de bâti : 20 000 × couverture / 0,6134 (20 000 pour la tile la plus bâtie du bloc). Médiane : 12 300 zombies par tile, environ **690 000 sur le bloc** à densité 1. Les densités 0,1 et 0,25 représentent une banlieue.

### Une tile : stock ou slot ([src/sim.rs](src/sim.rs))

Chaque tile du monde est un **stock** :

- **compteur** : jamais activée, seulement « n zombies » ;
- **sauvée** : a déjà été active ; sa sauvegarde est sur disque (ou en mémoire), avec un **journal de migration** ;
- **vivante** : ses zombies sont des entités, dans un **slot**.

Les slots forment un **pool réutilisé** (16 au départ, 64 au plus) : un slot libéré garde ses tableaux et les rend au suivant. Aucune allocation en régime établi. Un slot passe par quatre états : libre, chantier, actif, destruction. Analogie : un pool de connexions, pas une connexion ouverte par requête.

Une tile est **requise** quand un humain est dans la tile ou l'une de ses 8 voisines. Une tile active qui n'est plus requise depuis le **délai** (60 s par défaut) est désactivée.

### Activation : un chantier étalé

Le chantier d'une tile avance dans la phase des champs, en parallèle avec les autres tiles, avec un budget par tick. Ses étapes :

1. **Zone accessible** : la plus grande zone de rues connexe, où l'on place les zombies (aucun dans les bâtiments ni dans les cours fermées, B-02). Lue sur disque si elle est **précalculée** par la chaîne de données ; sinon calculée sur la tile seule (variante « zone locale »).
2. **Terrain de 2 m** des flow fields, avec un halo sur les tiles voisines, par tranches de lignes, avec un budget de 100 000 unités par tick (18 ticks pour une tile entière). Le résultat est identique au calcul d'un seul tenant (test).
3. **Zombies** : placement déterministe dans la zone à partir d'une graine (5 000 par tick), ou **restauration** de la sauvegarde (décodage, puis rejeu du journal de migration à 4 événements par tick).
4. **Naissance** : les entités sont créées par lots de 2 000, dans la phase de gestion (exclusive). Les zombies loin de tout humain naissent en veille profonde. Le budget du premier calcul de champ est estimé dès la fin du chantier.

Pendant le chantier, la tile reste **inerte** pour le reste du monde : ses zombies n'existent pas encore, et un zombie qui voudrait y entrer attend.

### Désactivation : un instantané normalisé

1. **Instantané** : la position de chaque zombie (quantifiée au 1/64 m en u16), son cap (1/256 de tour), la carte des corps (cases de 10 m) et les constructions avec leurs points de vie. Tous les zombies redeviennent **errants**. La liste est triée : deux tiles identiques donnent deux sauvegardes identiques, octet pour octet.
2. **Encodage** (zstd) et **écriture** sur disque dans un thread de fond, avec `fsync` en option. Une somme de contrôle est vérifiée à la lecture.
3. **Destruction** des entités par lots de 2 000. Une seule désactivation commence par tick.

La quantification est **idempotente** : une tile restaurée puis resauvegardée sans avoir bougé donne la même sauvegarde (test).

### Migration : un journal, pas une simulation ([src/sauvegarde.rs](src/sauvegarde.rs))

Toutes les 60 s de jeu, chaque tile inerte à portée d'un stimulus (incinérateur, rayon de 3 km) envoie 2 % de ses zombies vers sa voisine la plus proche du stimulus.

- Une tile **compteur** ajuste seulement ses compteurs.
- Une tile **sauvée** n'est pas relue : on ajoute un événement à son **journal**. Un *départ* retire les k zombies les plus proches du bord de sortie. Une *arrivée* place k zombies dans une bande de 20 m le long du bord d'entrée, à partir d'une graine. Le journal est rejoué à l'activation. Analogie : un journal d'écriture anticipée (WAL), rejoué au démarrage.
- Une tile qui dépasse **40 000 zombies** ne reçoit plus de migrants (saturation, comptée). Sans cette borne, l'incinérateur concentrait plus de 65 000 zombies dans une tile, au-delà de la capacité d'un slot.

### Frontières : passages entre tiles

Un zombie qui sort de sa tile **change de slot** dans une phase exclusive, en fin de tick, dans un ordre trié (slot, rang). Entre deux tiles actives, tout traverse la frontière :

- **perception** : la ligne de vue est testée sur le bâti du bloc ; chaque tile voit les humains des tiles voisines jusqu'à 500 m au-delà de son bord ;
- **séparation, relais et capacité** : un zombie près du bord consulte la grille de la tile voisine ;
- **réservation** des places, avec une clé de demande unique sur tout le bloc ;
- **flow fields** : échange des bordures entre voisines, horodaté (P-03), publié en double tampon.

Vers une tile qui n'est pas active (inerte ou en chantier), deux règles ont été mesurées :

- **bord réfléchissant** (retenu) : un zombie **sans cible** (errant) ne sort pas vers une tile non active, il est renvoyé. Un zombie qui **poursuit** une cible la suit ; s'il arrive dans une tile inerte, il est ajouté à son journal (événement exact) ;
- **bord traversable** : tout zombie qui sort entre dans la tile inerte.

Un passage vers un slot plein est refusé (compté).

### Tirs, barricades, corps

Pour éprouver la restauration des constructions et de la carte des corps, les humains peuvent **tirer** : toutes les secondes, chacun abat avec une chance sur deux le zombie le plus proche à moins de 25 m. Chaque zombie abattu ajoute un corps dans la case de 10 m. Les **barricades** ont des points de vie. Seuls les zombies en poursuite ou en relais les endommagent, et seulement dans une tile active ; une barricade détruite est retirée du terrain des flow fields.

### Scénarios ([src/main.rs](src/main.rs))

| Scénario | Description |
|---|---|
| **Colonne** | 500 humains traversent le bloc d'ouest en est le long du plus court chemin à pied, à 3 m/s (environ 1 h de jeu, 10,4 km) ; variante à 10 m/s. |
| **Base** | une base de 200 humains à 150 m à l'ouest d'une frontière de tile ; une patrouille de 20 humains fait l'aller-retour jusqu'à 150 m à l'est, à 1,5 m/s, avec des pauses de 60 s (variantes 20 et 180 s), pendant 30 min. Délai de désactivation de 0 à 300 s. |
| **Reprise** | une base de 300 humains qui tirent, entourée d'un anneau de barricades à 80 m, au centre d'une tile. Défense pendant 5 min, abandon à 10 m/s vers l'est, 10 min d'absence, retour et 5 min de défense. |
| **Migration** | un incinérateur au centre du bloc attire les zombies des tiles inertes à 3 km. 50 humains traversent le bloc à 10 m/s, attendent 30 min au bout et reviennent. |

### Déterminisme

Le résultat ne dépend ni du nombre de threads, ni du découpage en lots. Les phases parallèles ne lisent que l'état de la phase précédente ; les phases exclusives (gestion, frontières) traitent tout dans un ordre fixe ; les tirages aléatoires sont des hachages de (graine, tile, rang, tick). C'est vérifié :

- par les tests (`cargo test --release`, 14 tests, dont 5 de bout en bout sur un bloc synthétique de 3 × 2 tiles) : `conservation_et_restauration` (conservation à **chaque tick**, restaurations identiques, avec migration et tirs), `independant_du_decoupage` (lots de 1, 64 et 100 000), `meme_graine_memes_positions` (avec et sans étalement), `journal_de_migration`, `terrain_etale_identique`, `aller_retour_exact`, `quantification_idempotente` ;
- par la colonne `somme` de [resultats.csv](resultats.csv), identique pour 1, 4, 8 et 16 threads et des lots minimaux de 64, 1 024 et 100 000 (section H) ;
- en simulation, la conservation est vérifiée toutes les 20 s de jeu, et chaque restauration est comparée à la somme de contrôle de sa sauvegarde.

Reproduire : `./run.sh` (environ 1 h ; `ESSAI=1 ./run.sh` valide d'abord toutes les commandes en quelques secondes). Le script reconstruit le binaire release avant de mesurer. Images : `target/release/p04-activation --scenario colonne --observer images --fenetre 600 --etiquette colonne`, puis `python3 -I outils/ppm_vers_png.py images 2200` (environ 4 min). Résumé des résultats : `python3 -I outils/resume.py`.

### Paramètres de prototype

Ceux de P-03, plus : délai de désactivation de 60 s ; budgets du chantier (terrain 100 000 unités, placement 5 000 zombies, naissance et destruction par lots de 2 000, une désactivation par tick) ; préchauffage des tables de l'ECS pour 250 000 entités ; halo des humains de 500 m ; migration de 2 % par minute, saturée à 40 000 zombies par tile ; tir à 25 m une fois par seconde, une chance sur deux. Ce sont des hypothèses de mesure, pas des statistiques de jeu.

## Budgets de référence

Les mêmes que pour P-00 à P-03 : 25 ms par tick à ×1, 2,5 ms à ×10.

## Mesures

### Chaîne de données : zone accessible ([resultats-precalcul.csv](resultats-precalcul.csv))

| Mesure, par tile (55 tiles) | Médiane | Max |
|---|---|---|
| Zone locale (calculée sur la tile seule) | 45 ms | 60 ms |
| Zone globale (tout le bloc d'un coup) | 2,45 s pour les 55 tiles | |
| Rues accessibles par la seule zone globale | 0,8 % du bloc | 14 % d'une tile (x 651, y 6862) |
| Fichier de zone, bitmap LZ4 | 46 ko | 64 ko |
| Fichier de zone, cases exclues seulement | 26 ko | 41 ko |

La zone locale perd **0,8 % des rues** du bloc : des rues qui ne sont reliées au réseau qu'en passant par une tile voisine. Sur 7 tiles, la perte dépasse 1 % ; sur une, elle atteint 14 %. 10,9 % des cases libres ne sont pas accessibles (cours fermées, îlots).

### Banc tile par tile, un thread ([resultats-banc.csv](resultats-banc.csv))

55 tiles × 3 répétitions, à densité 1. Les 165 restaurations sont identiques à l'original.

| Étape | Médiane | p90 | Max |
|---|---|---|---|
| Terrain de 2 m avec halo | 6,2 ms (18 ticks à 100 000) | 6,4 ms | 6,6 ms |
| Terrain stocké en LZ4 : taille / décompression | 141 ko / 0,43 ms | 173 ko | 185 ko / 0,68 ms |
| Placement des zombies | 0,54 ms | 0,93 ms | 1,3 ms |
| Naissance des entités (préchauffées) | 0,38 ms | 0,53 ms | 2,1 ms |
| Destruction des entités | 0,59 ms | 0,81 ms | 1,8 ms |
| Instantané | 0,60 ms | 0,83 ms | 1,3 ms |

Format de la sauvegarde d'une tile (12 300 zombies en médiane, plus la carte des corps et les constructions) :

| Format | Taille médiane | Encodage | Décodage |
|---|---|---|---|
| Brut (f32) | 85 ko | 0,15 ms | 0,15 ms |
| Brut + zstd | 67 ko | 0,27 ms | 0,23 ms |
| Compact (quantifié) | 56 ko | 0,16 ms | 0,15 ms |
| Compact + LZ4 | 56 ko | 0,17 ms | 0,15 ms |
| **Compact + zstd** | **51 ko** (4,1 octets par zombie) | 0,24 ms | 0,28 ms |

Écriture : 0,10 ms ; avec `fsync`, 5,3 ms (max 15 ms) ; lecture : 0,014 ms (cache du système).

### Colonne à ×1, densité 1 ([resultats.csv](resultats.csv), section C)

Traversée complète : environ 11 tiles actives en moyenne, 15 au plus, jusqu'à 209 000 zombies vivants ; 24 activations, 27 désactivations ; 706 000 passages de frontière (90 au plus par tick).

| Variante | Médiane | p99 | p99 avec chantier | Max | Max entités / tick | Chantier |
|---|---|---|---|---|---|---|
| **Retenue** (étalée) | 2,97 ms | 4,14 | 6,0 | **7,3** | 1,5 ms | 27 ticks, 12,1 ms de travail |
| D'un seul tenant | 3,49 ms | 5,10 | 17,8 | **17,8** | 8,9 ms | 1 tick |
| Chantier étalé, entités d'un coup | 3,46 ms | 4,96 | 17,2 | 18,9 | 10,1 ms | 21 ticks |
| Sans préchauffage des tables | 2,96 ms | 4,13 | 6,2 | 11,4 | 8,4 ms (naissance 6,2) | 27 ticks |
| Vitesse 10 m/s | 3,01 ms | 4,27 | 5,6 | 6,0 | 1,2 ms | 27 ticks |

Phases du tick (retenue, moyenne) : gestion 0,01 ms, humains 0,20, grilles 0,50, champs et chantiers 0,55, perception 0,59, déplacement 0,91, admission 0,18, **frontières 0,01**.

Le chantier étalé coûte un peu plus de travail (12,1 ms contre 10,0) mais ne dépasse jamais 3 ms par tick. D'un seul tenant, il pèse jusqu'à 36 ms de travail sur un tick (réparti sur plusieurs threads quand plusieurs tiles s'activent ensemble). Les deux variantes « entités d'un coup » ont une médiane plus haute (3,5 ms contre 3,0) : la simulation y diverge (sommes différentes, les zombies naissent à d'autres ticks), mais l'écart n'est pas expliqué.

Variantes sur 20 min de jeu (9 activations) :

| Variante | p99 | Max | Chantier max par tick | Remarque |
|---|---|---|---|---|
| Zone précalculée, bord traversable | 3,37 | 6,0 | 3,0 ms | **15 196 zombies** passés dans des tiles inertes |
| Zone locale, étalée | 3,44 | **63 ms** | 159 ms de travail | la zone n'est pas étalée |
| Zone locale, d'un seul tenant | 3,35 | **71 ms** | 190 ms de travail | |

Sur ces 20 min, la zone précalculée tient le max à 6 ms ; la zone locale le multiplie par 10.

### Colonne au budget de ×10, banlieue (sections D et I)

| Densité | Variante | Médiane | p99 sans chantier | p99 avec chantier | Max | Ticks > 2,5 ms | Chantier |
|---|---|---|---|---|---|---|---|
| 0,1 | étalée, budgets de ×1 | 1,35 | 1,61 | 2,45 | 4,9 | 16 | 19 ticks |
| 0,1 | **étalée, budgets ÷ 4** | 1,35 | 1,59 | **1,88** | 4,3 | 12 | 73 ticks |
| 0,1 | d'un seul tenant | 1,37 | 1,71 | 13,9 | **13,9** | 27 | 1 tick |
| 0,25 | étalée, budgets de ×1 | 1,68 | 2,27 | 2,96 | 4,4 | 474 | 20 ticks |
| 0,25 | **étalée, budgets ÷ 4** | 1,63 | 2,04 | **2,47** | 5,3 | 71 | 79 ticks |
| 0,25 | d'un seul tenant | 1,73 | 2,33 | 11,8 | 11,8 | 321 | 1 tick |

À ×10, avec les budgets divisés par 4, le chantier dure 73 à 79 ticks : 3,7 à 4 s de jeu, soit 0,4 s de temps réel. Le max de 4,3 à 5,3 ms tombe **hors chantier** (le max avec chantier est de 2,0 et 3,2 ms) : ce sont des ticks isolés, sur une seule phase, que je range dans le bruit d'ordonnancement. À densité 0,25, 11 tiles actives dépassent déjà le budget de ×10 sans aucune activation (p99 de 2,0 à 2,3 ms) : c'est la charge, pas le chantier.

### Allers-retours en limite de tile (section E)

Base et patrouille, 30 min de jeu. La patrouille quitte le voisinage de la tile est pendant environ 260 s par cycle.

| Délai | Activations | Restaurations | Désactivations | p99 | Max |
|---|---|---|---|---|---|
| 0 s | 9 | 6 | 9 | 2,87 | 5,8 |
| 10 s | 9 | 6 | 9 | 2,87 | 6,0 |
| 30 s | 9 | 6 | 6 | 2,88 | 6,3 |
| **60 s** | 9 | 6 | 6 | 2,88 | 5,8 |
| 120 s | 9 | 6 | 6 | 2,88 | 6,0 |
| 300 s | 3 | 0 | 0 | 2,89 | 6,0 |
| 0 s, `fsync` | 9 | 6 | 9 | 2,87 | 5,1 |
| 60 s, pauses de 20 s | 12 | 9 | 9 | 2,88 | 5,6 |
| 60 s, pauses de 180 s | 6 | 3 | 6 | 2,86 | 5,9 |

Au plus 0,4 activation par minute. Les bascules ne coûtent rien de visible : p99 identique, max sous 6,3 ms dans tous les cas. Avec `fsync`, l'écriture prend 10,5 ms en moyenne, dans le thread de fond, sans effet sur le tick. Toutes les restaurations sont identiques à leur sauvegarde.

### Base avancée abandonnée puis reprise (section F, [chrono-reprise.csv](chrono-reprise.csv))

| Temps de jeu | Événement | Points de vie des barricades | Cases détruites |
|---|---|---|---|
| 0 s | défense, la tile est active | 1 307 940 | 0 |
| 300 s | abandon | | |
| 555 s | la tile devient **inerte** (sauvegarde) ; 8 167 zombies abattus jusque-là | 1 069 893 | 18 |
| 2 003 s | chantier : **restauration** | 1 069 893 | 18 |
| 2 004 s | active ; la base est reprise | 1 069 893 | 18 |
| 2 307 s | premiers dégâts après le retour | 1 069 891 | 18 |
| 2 498 s | fin | 1 065 253 | 19 |

Les barricades **ne bougent plus dès que les humains partent**, même tant que la tile reste active : aucun zombie n'est en poursuite. Elles sont restaurées avec exactement leurs points de vie. Sur l'ensemble du scénario : 33 activations, 18 restaurations, 18 contrôles de sauvegarde sur 18, constructions 18 sur 18, 36 331 zombies abattus et leurs corps restaurés. p99 2,66 ms, max 5,9 ms.

### Migration entre tiles inertes (section G, [chrono-migration.csv](chrono-migration.csv))

En 70 min de jeu, **233 484 zombies migrent** vers l'incinérateur ; 369 envois sont refusés par la saturation. 54 activations dont 27 restaurations avec journal ; le chantier dure jusqu'à 73 ticks (rejeu du journal) et 33 ms de travail au plus.

- p99 5,6 ms, max 11,4 ms. Les ticks au-dessus de 8 ms viennent de la **charge** : 12 tiles actives et plus de 250 000 zombies concentrés autour de l'incinérateur.
- Le max avec chantier vient d'une **naissance de 10 ms** : le nombre de zombies vivants (292 000) dépasse le préchauffage de 250 000.
- Avant correction, le placement des arrivées dans un coin de tile se faisait par tirage avec rejet, 1 000 fois plus lent qu'une énumération des cases libres. Un tick montait à 271 ms. Le rejeu est maintenant étalé à 4 événements par tick.

### Déterminisme (section H)

Scénario de reprise à densité 0,25 (33 activations, 18 restaurations, 8 154 zombies abattus) :

| Threads | Lot minimal | Médiane | Somme |
|---|---|---|---|
| 16 | 1 024 | 1,31 ms | `912a1f8ab7368df2` |
| 8 | 1 024 | 1,30 ms | `912a1f8ab7368df2` |
| 4 | 1 024 | 1,69 ms | `912a1f8ab7368df2` |
| 1 | 1 024 | 4,88 ms | `912a1f8ab7368df2` |
| 16 | 64 | 1,31 ms | `912a1f8ab7368df2` |
| 16 | 100 000 | 2,25 ms | `912a1f8ab7368df2` |

Toutes les sommes de la campagne sont aussi identiques à celles d'une première campagne complète, faite avant la correction de `clear_trackers` (voir l'analyse).

### Images ([images/](images))

[colonne-0600s.png](images/colonne-0600s.png), [colonne-1800s.png](images/colonne-1800s.png), [colonne-3600s.png](images/colonne-3600s.png) : le bloc à 5 m par pixel pendant la traversée, nord en haut. Gris sombre : tiles inertes, le brouillard de guerre ; elles n'ont pas de zombies à l'écran. Couleur : passages de zombies pendant les 10 dernières minutes, du jaune (peu) au violet (beaucoup). Vert : les humains. Pointillés : limites des tiles. La couronne de 3 × 6 tiles actives suit la colonne ; une tile qui vient de s'activer est plus claire, faute de passages accumulés.

## Analyse

**1. Activation et désactivation sont exactes.** La conservation est vérifiée à chaque tick dans les tests, et toutes les 20 s dans chaque simulation de la campagne : aucun écart. Les 165 restaurations du banc et les 201 restaurations des simulations sont identiques à leur sauvegarde, constructions comprises. Le résultat est identique de 1 à 16 threads et pour des lots de 64 à 100 000. Trois mécanismes y suffisent :

- un instantané **normalisé** (quantifié, trié, tous errants), donc une seule représentation par état ;
- des tirages aléatoires qui sont des **hachages** (graine, tile, rang, tick), sans générateur partagé ;
- des phases **exclusives et triées** pour tout ce qui change l'appartenance d'un zombie (naissance, destruction, passage de frontière, tir).

**2. L'activation doit être étalée, et elle peut l'être sans perte.** D'un seul tenant, l'activation fait monter le tick à 12 à 18 ms : sans danger à ×1, mais 5 fois le budget de ×10. Étalée, elle ne dépasse jamais 3 ms par tick, et le p99 des ticks avec chantier tombe de 17,8 à 6,0 ms à ×1. Il faut étaler **les trois postes** :

- le **terrain** de 2 m (6 ms d'un tenant), par tranches de lignes ;
- la **naissance et la destruction** des entités (jusqu'à 10 ms d'un tenant pour 20 000 entités), par lots de 2 000 ;
- les **désactivations** simultanées : une seule commence par tick.

Étaler seulement le chantier et créer les entités d'un coup laisse le max à 19 ms.

**3. Trois pièges de l'ECS, invisibles dans P-00 à P-03, coûtaient plus cher que l'activation elle-même.**

- **`clear_trackers`** : une application Bevy complète vide à chaque frame les journaux de « composants retirés ». Notre boucle, qui pilote le `World` à la main, ne le faisait pas : un `despawn` sur des centaines de milliers coûtait 5 à 14 ms, quand le journal doublait de taille. C'est un journal qu'on ne fait jamais tourner. Corrigé : le max de la colonne retenue passe de 14,5 à 7,3 ms, celui de ×10 de 6,7 à 4,3 ms, et la mémoire baisse de 8 à 18 %, sans aucun changement de résultat.
- **Préchauffage des tables** : quand le nombre d'entités franchit un palier, Bevy réalloue et recopie ses colonnes (naissance de 6,2 ms au lieu de 1,5). Créer puis détruire 250 000 entités au démarrage supprime ces recopies, tant que le jeu reste sous ce nombre ; au-delà, elles reviennent (migration : naissance de 10 ms à 292 000 zombies). C'est l'équivalent d'une préallocation de pool.
- **Premier champ d'une tile neuve** : son coût n'était pas estimé, il tombait d'un seul tenant au tick suivant le chantier (pics de 4 à 6 ms à ×10). Il est maintenant estimé dès la fin du chantier, à partir du nombre de cases libres.

**4. La zone accessible doit venir de la chaîne de données.** Calculée à l'activation, elle coûte 45 à 60 ms sur un thread et n'est pas étalée : le tick monte à 63–71 ms. Elle est aussi fausse : sans les voisines, elle perd 0,8 % des rues du bloc, et 14 % sur une tile. Calculée une fois pour tout le bloc (2,5 s pour 55 tiles), elle tient en 26 à 46 ko par tile urbaine dense. Le **terrain** de 2 m, en revanche, ne se stocke pas : 141 ko par tile pour gagner 6 ms de calcul étalé, qui ne coûte rien de visible.

**5. La sauvegarde est petite et rapide : compact + zstd, environ 4 octets par zombie.** 51 ko pour une tile de 12 300 zombies, corps et constructions compris ; 0,6 ms d'instantané sur le thread principal, puis 0,25 ms d'encodage et 0,1 ms d'écriture en tâche de fond. Le gain de zstd sur LZ4 est de 8 % (51 contre 56 ko), pour un décodage aussi rapide. Le `fsync` coûte 5 à 15 ms : en tâche de fond, cela ne touche pas le tick, mais il ne sert à rien tant que la sauvegarde de partie n'en dépend pas. Ordre de grandeur : 1 000 tiles visitées font 50 Mo.

**6. Le délai de désactivation n'a pas d'enjeu de coût.** Une bascule complète (sauvegarde, destruction, restauration) ne se voit pas dans le p99. Le délai ne règle donc que le **gaspillage** : avec un délai plus court que l'absence d'une patrouille, la tile bascule à chaque passage, sinon elle reste active. 60 s convient pour une colonne. Les fréquences mesurées restent basses : 0,4 activation par minute pour une colonne à pied, 1,3 en véhicule à 10 m/s, 0,8 pour une base reprise.

**7. Les frontières : la phase coûte moins de 0,01 ms, et la règle du bord compte plus que son coût.** 706 000 passages en une heure, 90 au plus par tick. Mais avec un bord traversable, les errants **fuient en sens unique** vers les tiles inertes : ils y entrent par diffusion, et n'en ressortent jamais, puisqu'une tile inerte ne simule personne. 15 196 zombies en 20 min, environ 10 % de ceux de la couronne : la couronne se vide, et les tiles inertes voisines se remplissent. Le **bord réfléchissant** pour les errants supprime la fuite ; un zombie en poursuite traverse toujours.

**8. La migration par journal fonctionne, et a besoin d'une borne.** 233 000 migrants en 70 min, sans relire aucune sauvegarde. Le rejeu à l'activation est étalé (4 événements par tick). Sans saturation, un incinérateur concentre plus de 65 000 zombies dans une tile : au-delà de la capacité d'un slot, et au-delà de ce que la réservation des places peut y loger.

**9. Le déterminisme ne coûte rien de mesurable.** Les deux phases exclusives et triées (gestion, frontières) coûtent ensemble 0,02 ms par tick en moyenne.

### Limites

- **Densité approchée** par la part de bâti, faute de données INSEE ; un seul bloc, parisien. Les tiles rurales (zone presque entière, sauvegardes plus petites) ne sont pas mesurées.
- **Carte résidente** : le bloc de 55 tiles tient en mémoire. À l'échelle de la métropole, le bâti et les zones devront être chargés à la demande, ce qui n'est pas mesuré.
- **Humains sommaires** : ils ne réagissent que par le tir ; pas d'infection, pas de bâtiments occupés, pas de véhicules. Seules constructions : des barricades.
- **Un seul champ** (« proies ») : le champ « bruits » de P-03 est retiré ; la migration vers l'incinérateur est un stimulus abstrait, pas un champ.
- **Halo des voisines** : quand une barricade est détruite dans une tile, le terrain des voisines (dans leur halo) n'est pas recalculé avant leur prochaine activation.
- **Préchauffage fixe** de 250 000 entités : au-delà, les pics de naissance reviennent. Il faudrait le dimensionner sur le maximum attendu, ou préchauffer par paliers.
- **Pool de 64 slots au plus** (6 bits dans la clé de réservation) et 65 000 zombies par slot (rangs sur 16 bits) : des limites du prototype, larges devant les budgets mesurés (40 tiles calmes au plus à ×1).
- **Sauvegarde de partie** : seules les sauvegardes de tiles sont mesurées, pas une sauvegarde complète, ni son chargement.
- **Bruit de mesure** : des ticks isolés de 1 à 4 ms au-dessus du p99, sur une seule phase, hors chantier. Un tick de 28 ms hors chantier, dans la première campagne (base, pauses de 20 s), ne s'est pas reproduit.

## Conclusion

- L'activation et la désactivation des tiles sont **exactes** et **déterministes** : conservation à chaque tick, restaurations identiques octet pour octet, résultat indépendant des threads et des lots.
- **Activation étalée** sur environ 27 ticks à ×1 (1,4 s de jeu), environ 75 ticks avec les budgets de ×10 : jamais plus de 3 ms par tick. À ×1, une colonne dense traverse 10 km sans tick au-dessus de 7,3 ms ; à ×10 en banlieue, le chantier n'ajoute que 0,3 à 0,4 ms au p99.
- La **zone accessible** se précalcule dans la chaîne de données ; le terrain de 2 m se recalcule à l'activation.
- **Sauvegarde compacte + zstd** : environ 4 octets par zombie, 51 ko par tile dense, écrite en tâche de fond.
- **Bord réfléchissant** pour les zombies sans cible ; **saturation** de la migration.
- Côté ECS : vider les journaux de l'ECS à chaque tick, préchauffer les tables, estimer le coût du premier champ.

## Décisions

Prises à l'issue du prototype et reportées dans les documents de conception :

- **Activation étalée** : zone lue sur disque, terrain par tranches, placement ou restauration par lots, naissance et destruction par lots d'environ 2 000 entités, une désactivation commencée par tick. Budgets divisés par le facteur d'accélération pour garder le même temps réel ([A-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-02--accélération-du-temps), [F-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation)).
- **Zone accessible** des zombies **précalculée par la chaîne de données** sur tout le territoire, et **stockée par tile dans les données distribuées** ; le terrain des flow fields est recalculé à l'activation ([B-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-01--chaîne-de-traitement-des-données-ign)).
- **Sauvegarde d'une tile** : instantané normalisé (positions au 1/64 m, cap au 1/256 de tour, tous errants, trié), compressé en zstd, avec somme de contrôle, écrit en tâche de fond, **un fichier par tile**, sans `fsync` pendant la partie. Les fichiers de tiles sont **immuables** : une nouvelle version est un nouveau fichier. Une **sauvegarde de partie** est un manifeste (la liste des versions de tiles) plus des liens physiques vers ces fichiers, avec un `fsync` à ce moment-là seulement ; un crash peut perdre les dernières désactivations, jamais une sauvegarde de partie. Sauvegarde refusée si les liens physiques sont impossibles (autre système de fichiers, FAT32 ou exFAT). Analogie : un instantané copy-on-write ([A-13](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-13--sauvegardes)).
- **Constructions des tiles inertes résidentes en mémoire** (choix délégué) : quelques octets par construction, nécessaires à l'affichage sous le brouillard et à la planification du joueur ; seuls les zombies et la carte des corps vont sur disque ([A-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-01--nombre-de-zombies--simulation-par-tiles)).
- **Migration** par journal sur les tiles sauvées, rejoué et étalé à l'activation ; arrivées dans une bande de 20 m le long du bord d'entrée. **Saturation** : une tile inerte qui compte 40 000 zombies n'en reçoit plus ; les envois refusés restent dans leur tile de départ.
- **Capacité d'une tile active : 65 000 zombies** ; au-delà, un passage de frontière vers elle est refusé : les zombies attendent au bord, dans leur tile, comme devant une case pleine (A-05).
- **Bord réfléchissant** : un zombie sans cible ne sort pas vers une tile non active ; un zombie en poursuite traverse ([ZOMBIES.md](https://github.com/labatailledefrance/documentation/blob/main/game-elements/zombies/ZOMBIES.md#déplacement)).
- **Pool de slots** réutilisés pour les tiles actives ; **préchauffage** des tables de l'ECS pour **800 000 zombies**, ce que le budget de ×1 permet (environ 40 tiles × 20 000) ; **`clear_trackers`** (ou la boucle d'`App` de Bevy) à chaque tick.
- **Délai de désactivation** de 60 s maintenu : sans enjeu de coût.

## Questions ouvertes

- (aucune : les décisions sont reportées dans les documents de conception)
