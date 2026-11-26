# P-09 — Recherche de chemin routière

Prototype jetable. Problèmes : [A-07](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-07--recherche-de-chemin), [B-03](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-03--charge-limite-des-ponts), [A-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-02--accélération-du-temps), [A-03](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-03--unités-dispersées-sur-tout-le-territoire), [A-12](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-12--déterminisme-et-multijoueur), [A-13](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#a-13--sauvegardes), [B-01](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-01--chaîne-de-traitement-des-données-ign), [B-05](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#b-05--destructions-et-modifications-du-terrain), [F-02](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation). Fiche du prototype : [prototypes/README.md](../README.md#p-09--recherche-de-chemin-routière). Premier prototype sur le **réseau routier** des tiles de [P-07](../p07-chaine-manche/README.md).

## Hypothèse testée

Les véhicules du joueur (convois logistiques, blindés, génie) roulent sur le réseau routier réel : celui de la Manche aujourd'hui, celui de la France demain. Une recherche de chemin **hiérarchique**, mise à jour à chaque modification du réseau (A-07), doit donner des itinéraires exacts, par classe de véhicule et avec la charge limite des ponts (B-03), assez vite pour que des milliers de convois replanifient à ×100 sans peser sur le tick. P-09 doit établir :

1. le **graphe** : construit à partir des morceaux de la section `reseau` des tiles, recousus ; ce qui se précalcule dans la chaîne ;
2. la **charge des ponts** : l'heuristique décidée en P-07, appliquée à tous les ponts de la Manche ;
3. les **classes de véhicules** : une métrique par classe sur un même prétraitement, ou un prétraitement par classe ;
4. l'**algorithme** : Dijkstra, Dijkstra bidirectionnel, A\*, ALT, contraction hierarchies (CH) et CH personnalisables (CCH) ;
5. les **modifications** : pont détruit, routes coupées, réouverture ; mise à jour contre recalcul complet ; itinéraires invalidés et replanifiés ;
6. le **passage à l'échelle** : de la Manche à la France ;
7. le **déterminisme** : même requête, même itinéraire, quels que soient les threads et l'ordre de construction ;
8. le **raccord avec le jeu** : itinéraire suivi en tile active et en tile inerte ; sauvegarde.

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4, SSD NVMe, Linux 7.0. Rust 1.99, compilation portable (sans `target-cpu=native`), `lto = "thin"`. Mesures du 2026-11-24. Rien d'autre ne tournait pendant la campagne. Micro-banc `#[ignore]` à la place de `perf`, interdit sur la machine.

## Méthode

### Données

Aucun téléchargement : les tiles de P-07 (`data/generated/p07-manche`, 14 076 tiles ; `data/generated/p07-paris`, 105 tiles), sections `reseau` et `ponts`. La BD TOPO de la Manche et de Paris (`data/sources/`) n'a servi qu'à vérifier des attributs absents des tiles (nœuds, interdictions de tourner, largeurs).

Constats sur la BD TOPO 3.5 :

- **Pas de table de nœuds** : les tronçons sont coupés aux intersections, et deux tronçons qui se rejoignent partagent une extrémité. Les intersections se retrouvent donc par les coordonnées des extrémités.
- **Interdictions de tourner** : table `non_communication` (tronçon d'entrée, tronçons de sortie), 152 dans la Manche, 1 378 à Paris. Absente des tiles.
- **Largeur de chaussée** : ce sont des **classes**, pas des mesures. Dans la Manche, 47 000 routes à une chaussée ont 3 m (une voie) et 67 000 en ont 4 (deux voies). Les routes à chaussées séparées (2 × 2 voies, bretelles) sont décrites chaussée par chaussée (souvent 3 à 3,5 m). Les chemins et les routes empierrées n'ont presque jamais de largeur.
- **Sens de circulation** : 15 700 tronçons à sens unique dans la Manche, 52 900 à Paris (35 % des tronçons).
- **Tronçons fictifs** : 52 dans la Manche (liaisons à travers une place ou un parking). L'attribut est lu par P-07 mais n'est pas écrit dans les tiles.

### Le graphe ([src/graphe.rs](src/graphe.rs), [src/tuiles.rs](src/tuiles.rs))

1. **Lecture** : les paquets de P-07 sont lus en parallèle ; chaque tile est vérifiée par son empreinte BLAKE3, puis ses sections `reseau` et `ponts` sont décompressées. Les coordonnées deviennent des **entiers globaux au 1/16 m** : coordonnée locale quantifiée plus centre de la tile. Le centre étant un nombre entier de mètres, deux tiles voisines donnent exactement le même entier pour un même point de coupe.
2. **Recousure** : les morceaux d'un même tronçon (identifiant BD TOPO) sont triés par rang et mis bout à bout quand les rangs se suivent et que les extrémités coïncident. Un trou dans les rangs (partie du tronçon hors du département) laisse deux fragments. La même fonction recoud des morceaux, des fragments de tiles ou des fragments de départements : f(A ∪ B) = f(f(A) ∪ f(B)). Analogie : le réassemblage des fragments IP par (identifiant, décalage), qui ne dépend pas de l'ordre d'arrivée.
3. **Nœuds** : les extrémités des fragments, identifiées par leurs coordonnées entières. Les points de coupe aux bords des tiles ne deviennent jamais des nœuds : ils sont recousus avant.
4. **Réseau carrossable** : le graphe de recherche garde les natures qu'un véhicule peut emprunter (autoroutes, routes à 1 ou 2 chaussées, bretelles, ronds-points, routes empierrées, chemins). Les sentiers, escaliers, pistes cyclables et bacs restent dans les tiles (contrôle de longueur) mais sortent du graphe.
5. **Numérotation canonique** : nœuds triés par (y, x), arêtes par (identifiant, rang de début). Le graphe ne dépend ni de l'ordre de lecture des tiles ni du nombre de threads.
6. **Arcs** : chaque arête donne deux arcs (aller, retour), toujours présents dans la topologie. Le sens interdit, la classe de véhicule ou la coupure se traduisent par un poids infini : la topologie ne change jamais quand une métrique change.
7. **Ponts** : un pont de la section `ponts` est associé aux arêtes au-dessus du sol dont il a un sommet à moins de 5 m (règle de P-07). Une arête portée par plusieurs ponts prend la plus faible charge. Une arête au-dessus du sol sans pont reçoit un **ouvrage implicite**, avec la même heuristique.

### Charge des ponts (B-03, [src/classes.rs](src/classes.rs))

L'heuristique décidée en P-07, appliquée à chaque ouvrage. Les paliers s'inspirent des convois du règlement français des ponts routiers (convois militaires Mc 80 et Mc 120 sur le réseau principal, à vérifier) :

| Importance de la route portée | Palier de départ |
|---|---|
| 1 ou 2 (réseau national, grands axes) | 120 t |
| 3 | 80 t |
| 4 | 44 t (poids lourd réglementaire) |
| 5 | 26 t |
| 6 et plus | 12 t |

Puis, dans l'ordre :

- **restriction réelle** de la BD TOPO (pont ou tronçon) : elle remplace tout le reste ;
- **valeur écrite à la main** (identifiant BD TOPO du pont) : prévue, mais la liste est vide (voir *Limites*) ;
- **une voie** (classe de largeur sous 3,5 m) : un palier de moins ; sous 2,5 m, passerelle de 4 t. Les chaussées séparées (largeur donnée par chaussée) sont exemptées ;
- **grand ouvrage sur l'eau** (100 m et plus) d'une route d'importance 4 ou plus (ponts suspendus, ouvrages anciens) : un palier de moins ;
- **ponceau** (moins de 10 m) : un palier de plus ;
- **passage supérieur** sur une route ou une voie ferrée (construit à une norme récente) : au moins 44 t.

Toutes ces valeurs sont des **hypothèses de mesure**.

### Classes de véhicules (hypothèses de mesure)

Masses et gabarits de l'arsenal réel, à vérifier :

| Classe | Masse | Largeur | Hauteur | Natures permises | Vitesse maximale |
|---|---|---|---|---|---|
| Voiture (VL civil ou militaire léger) | 2 t | 1,8 m | 1,6 m | routes revêtues, routes empierrées | 130 km/h sur autoroute, 40 sur route empierrée |
| Camion (porteur logistique chargé, type PPLD) | 32 t | 2,6 m | 3,6 m | toutes, chemins compris | 90 ; 20 sur chemin |
| Porte-char chargé d'un Leclerc | 110 t | 3,7 m | 4,2 m | routes revêtues seulement | 70 ; 60 sur route à une chaussée |
| Leclerc sur route | 60 t | 3,7 m | 2,5 m | toutes, y compris les tronçons « physiquement impossibles » pour un VL quand ce sont des chemins | 50 ; 25 sur chemin |

Règles de refus : nature, accès « physiquement impossible », largeur de chaussée connue plus étroite que le véhicule (véhicules à roues, hors chaussées séparées ; un chenillé déborde sur les accotements), restrictions de hauteur, de largeur et de poids de la BD TOPO, charge et largeur du pont. Vitesse : celle de la BD TOPO (`vitesse_moyenne_vl`), sinon une valeur par nature, bornée par la classe. Le temps d'une arête est en **millisecondes entières**, arrondi au-dessus, calculé sans flottant : ⌈3,6 × longueur en mm / vitesse en km/h⌉.

Deux métriques de comparaison, sans limite de charge (`leclerc-libre`, `porte-char-libre`), isolent l'effet des ponts. La métrique `voiture-code` respecte **tous** les sens uniques. Les métriques principales ne respectent que ceux des **chaussées séparées** (autoroutes, routes à 2 chaussées, bretelles) : dans une France vide, rien n'empêche un convoi de prendre une rue à contresens, mais on ne remonte pas une autoroute par la bretelle de sortie.

### Les méthodes ([src/algos.rs](src/algos.rs), [src/hier.rs](src/hier.rs))

| Méthode | Prétraitement | Ce qu'il faut refaire après une modification |
|---|---|---|
| **Dijkstra** (référence exacte) | aucun | rien |
| **Dijkstra bidirectionnel** | aucun | rien |
| **A\*** (borne : distance à vol d'oiseau à la vitesse maximale de la classe) | aucun | rien |
| **ALT** (16 repères choisis par la méthode du plus éloigné ; bornes par inégalité triangulaire) | 32 Dijkstra complets par classe | rien si l'on ne fait que couper (les bornes restent valides) ; tout si un raccourci apparaît |
| **CH** (crate `fast_paths` 1.0.0) | contraction complète, **par classe** | la contraction de la classe (au mieux avec l'ordre déjà calculé) |
| **CCH** (crate `cch` 0.3.0, portage en Rust pur de RoutingKit) | **ordre** par dissection emboîtée (inertial flow) et **structure**, une fois pour la topologie ; puis une **personnalisation** par classe | la personnalisation, **partielle** : seulement les raccourcis qui dépendent des arcs modifiés |

**Analogie** pour la CCH : la dissection emboîtée découpe le réseau en zones séparées par quelques carrefours de frontière, récursivement, comme une hiérarchie d'aires OSPF. La structure (quels raccourcis existent entre carrefours de frontière) ne dépend que de la topologie. Les coûts des raccourcis se recalculent quand un lien change de coût, sans toucher à la topologie. Une requête ne visite que les carrefours de frontière de ses deux extrémités, en remontant l'arbre des zones (*arbre d'élimination*), sans file de priorité.

La **personnalisation partielle** n'existe pas dans la crate : elle est écrite dans ce prototype. Un nœud marqué recalcule tous ses raccourcis montants (base, puis triangles inférieurs) avec la boucle de la personnalisation complète. Si l'un change, les nœuds dont un raccourci dépend de celui-ci sont marqués. Les nœuds sont traités par rang croissant, puisqu'un raccourci ne dépend que de raccourcis de rang plus bas. Le résultat est **comparé bit à bit** à une personnalisation complète après chaque modification.

**Requête CCH** : chemin en nœuds (crate), puis en arcs du graphe (arc de poids minimal entre deux nœuds consécutifs, égalités départagées par l'identifiant de l'arc). **Dijkstra et A\*** : tas ordonné par (clé, nœud), prédécesseur remplacé seulement par un chemin strictement meilleur.

### Scénarios ([run.sh](run.sh))

| Scénario | Mesure |
|---|---|
| 1. Construction | Manche depuis les tiles : temps, mémoire, nœuds, arêtes, composantes, longueur contre celle de P-07, recousure aux bords des tiles et entre deux pseudo-départements (moitiés nord et sud recousues séparément puis ensemble) |
| 2. Requêtes | 10 000 paires aléatoires (graine fixe) dans la composante fortement connexe de la voiture ; quatre classes ; Dijkstra, bidirectionnel, A\* et ALT chronométrés sur 1 000 paires, CH et CCH sur les 10 000, une requête à la fois sur un thread ; exactitude contre Dijkstra sur les 10 000 ; trajets typiques (Cherbourg → Saint-Lô, Cherbourg → Granville, La Hague → Mont-Saint-Michel, base aéronavale de Maupertus → port de Cherbourg), lieux accrochés par classe |
| 3. Charge limite | trajets qui changent du camion au Leclerc et au porte-char ; effet des seuls ponts (métriques sans limite) ; trajets impossibles ; ponts refusés |
| 4. Modifications | le pont d'importance 1 ou 2 le plus emprunté par les camions détruit ; 1, 10, 100 et 1 000 routes empruntées coupées ; réouverture. Mise à jour partielle, personnalisation complète, nouvelle préparation des CH ; trajets touchés replanifiés et comparés à Dijkstra sur le réseau modifié |
| 5. Charge de jeu | 1 000 convois (60 % camions, 20 % voitures, 10 % porte-chars, 10 % Leclerc), 100 routes coupées sur leurs trajets : mise à jour, détection des convois touchés, replanification dans la boucle (3 ms par tick, P-04) ou en tâche de fond (4 et 15 threads) |
| 6. Paris | mêmes mesures sur un réseau urbain dense (rues à sens unique, petits tronçons) ; sens du code de la route contre chaussées séparées |
| 7. Déterminisme | par jeu de données, cinq exécutions (16 threads, 1 thread, ordres de lecture mélangé et inverse, nouvelle exécution à 16 threads) : empreintes des fragments, du graphe, de l'ordre CCH, des sept métriques personnalisées, des trajets CCH (2 000 paires) et Dijkstra (500) |
| 8. Suivi en jeu | trajet du camion Cherbourg → Saint-Lô : géométrie par les morceaux des tiles, positions locales en f32, position en tile inerte, taille d'un itinéraire et d'une liste de modifications |
| 9. Fichier livré | graphe compact et ordre CCH, comme la chaîne les livrerait ; chargement en jeu (structure reconstruite depuis l'ordre, personnalisation) ; ajout d'une arête (nouvelle traversée) |

### Paramètres de prototype

Ce sont des **hypothèses de mesure**, pas des statistiques de jeu : classes et paliers de charge ci-dessus ; vitesses par défaut (autoroute 110 km/h, 2 chaussées 90, une chaussée 60 et 40 en ville, bretelle 50, rond-point 30, route empierrée 25, chemin 15) ; association d'un pont à moins de 5 m ; 16 repères pour ALT ; budget de 3 ms par tick (P-04).

Reproduire : `./run.sh` (environ 15 min) ; `ESSAI=1 ./run.sh` valide toutes les commandes en 2 min. Images seules : `./run.sh images`. Tests : `cargo test --release` (3 tests) ; micro-banc : `cargo test --release banc -- --ignored --nocapture`.

## Budgets

- **Tick** : 3 ms par tick pour tout calcul étalé (P-04), et beaucoup moins à ×100 (les budgets se divisent par le facteur d'accélération, A-02) : la recherche de chemin doit pouvoir tourner **en tâche de fond**.
- **Requête** : quelques centaines de microsecondes au plus pour un trajet départemental, pour que des milliers de convois replanifient en une fraction de seconde.
- **Mise à jour** : une destruction (pont, gravats, épave) se répercute en quelques millisecondes, sans bloquer le tick.
- **Mémoire** : le graphe national et ses métriques doivent tenir en quelques Go au plus dans les 32 Go de la machine.

## Mesures

Dernière exécution complète de `./run.sh` ; chiffres bruts dans [resultats/](resultats/). Requêtes chronométrées une à la fois, sur un thread.

### 1. Construction

| | Manche | Paris |
|---|---|---|
| Tiles lues | 14 076 | 105 |
| Graphe carrossable : nœuds, arêtes | 153 263, 181 138 | 23 111, 32 605 |
| Composantes (part de la plus grande) | 208 (99,5 %) | 84 |
| Longueur totale, contre P-07 | 31 088 km, écart 1,9 × 10⁻⁶ | 2 790 km, écart 2,2 × 10⁻⁶ |
| Recousure aux bords des tiles | 38 779 jointures, 0 doublon, 0 rupture, 19 trous (bords du département) | idem, 0 doublon, 0 rupture |
| Deux pseudo-départements recousus séparément puis ensemble | même graphe | même graphe |
| Ponts associés aux tronçons (par la géométrie, à moins de 5 m) | 2 286 | 218 |
| Chargement complet (tiles → graphe) | 1,3 s | 0,15 s |
| Ordre CCH (dissection emboîtée), structure | 4,4 s, 49 ms | 0,6 s, 11 ms |
| Personnalisation d'une classe (16 threads, 1 thread) | 7 ms, 20 ms | 4,5 ms |
| CH classiques : préparation d'une classe (sans, avec l'ordre déjà calculé) | 0,3 à 2,4 s, 0,1 à 0,4 s | |
| Mémoire maximale, tout compris | 880 Mo | 169 Mo |

### 2. Requêtes

10 000 paires aléatoires (1 000 pour les méthodes sans prétraitement) ; exactitude contre Dijkstra : **0 écart** pour toutes les méthodes et les quatre classes, dans les deux jeux de données.

| Manche, médiane (p99) | Voiture | Camion | Porte-char | Leclerc |
|---|---|---|---|---|
| Dijkstra | 6,8 ms (14,4) | 8,9 ms (19,4) | 0,2 ms (6,3) | 8,8 ms (18,9) |
| Bidirectionnel | 5,1 ms (15,7) | 6,3 ms (21,1) | — | 6,9 ms (21,0) |
| A\* | 4,7 ms (17,8) | 4,3 ms (22,8) | 0,3 ms (9,0) | 2,7 ms (20,3) |
| ALT | 1,0 ms (7,9) | 1,6 ms (12,0) | 0,4 ms (27,9) | 1,5 ms (13,0) |
| CH | 30 µs (48) | 31 µs (50) | 3 µs (23) | 32 µs (51) |
| **CCH**, chemin en arcs compris | **49 µs (100)** | **51 µs (108)** | **6 µs (86)** | **54 µs (117)** |

À Paris : Dijkstra 1,0 ms en médiane ; CH 15 à 18 µs ; CCH 32 à 34 µs (p99 48 à 57 µs). Les temps du porte-char sont faibles parce que la plupart des paires sont impossibles pour lui (7 774 sur 10 000 dans la Manche) et s'arrêtent tôt.

Trajets typiques, Manche ([images/](images/)) :

| Trajet | Voiture | Camion | Porte-char | Leclerc |
|---|---|---|---|---|
| Cherbourg → Saint-Lô | 68 min, 80 km | 71 min, 80 km | 87 min, 89 km | 101 min, 79 km |
| Cherbourg → Granville | 105 min, 106 km | 105 min, 106 km | 172 min, 160 km | 137 min, 105 km |
| La Hague → Mont-Saint-Michel | 142 min, 202 km | 153 min, 173 km | 198 min, 206 km | 204 min, 163 km |

### 3. Charge limite

| Manche | Arêtes permises | Refus : nature, accès, largeur, hauteur, poids, pont | Ponts de la BD TOPO refusés |
|---|---|---|---|
| Voiture | 156 304 | 24 176, 658, 0, 0, 0, 0 | 0 |
| Camion | 179 816 | 0, 658, 21, 31, 29, 583 | 575 |
| Porte-char | 82 640 | 52 173, 655, 44 640, 39, 17, 974 | 945 |
| Leclerc | 178 950 | 0, 655, 12, 8, 31, 1 482 | 1 460 |

Sur 9 920 paires, passer du camion au Leclerc change 8 792 chemins (distance quasi inchangée, durée +34 % en moyenne et +64 % au p99, le char roulant moins vite) et en rend 87 impossibles ; au porte-char, 7 694 deviennent impossibles. Les seuls ponts (métriques « libres », sans limite de charge) expliquent 120 trajets impossibles du Leclerc sur 164. Le Leclerc refuse 497 des 1 260 ponts que les camions empruntent.

### 4. Modifications

Personnalisation **partielle**, comparée bit à bit à une personnalisation complète après chaque modification : **identique** dans tous les cas, réouverture comprise (elle rend exactement la métrique d'origine). Trajets touchés replanifiés : **0 écart** avec Dijkstra sur le réseau modifié.

| Manche, voiture | Partielle | Arcs recalculés | Complète | CH, nouvelle préparation avec l'ordre | Trajets touchés (sur 10 000) | Replanification |
|---|---|---|---|---|---|---|
| Pont le plus emprunté par les camions détruit | 0,8 ms | 4 389 | 8,0 ms | 300 ms | 1 554 | 111 ms (72 µs par trajet) |
| 1 route coupée | 0,06 ms | 532 | 8,1 ms | 291 ms | 3 | 0,3 ms |
| 10 routes | 1,3 ms | 9 147 | 8,2 ms | 300 ms | 72 | 4,6 ms |
| 100 routes | 5,5 ms | 52 151 | 7,2 ms | 284 ms | 4 546 | 283 ms |
| 1 000 routes | 10,4 ms | 129 228 | 7,6 ms | 302 ms | 9 239 | 508 ms |

Mêmes ordres de grandeur pour les autres classes et à Paris (100 coupures : 5 ms en partielle, 4,5 ms en complète). Au-delà d'une centaine de coupures simultanées, la personnalisation complète, parallèle, devient plus rapide que la partielle, séquentielle.

### 5. Charge de jeu

1 000 convois (60 % camions, 20 % voitures, 10 % porte-chars, 10 % Leclerc), 100 routes coupées sur leurs trajets.

| | Manche | Paris |
|---|---|---|
| Calcul initial des 1 000 itinéraires (16 threads) | 120 ms | 16 ms |
| Mise à jour des 4 métriques | 22 ms | 21 ms |
| Détection des convois touchés (index arête → convois) | 0,06 ms (index : 24 ms) | 0,06 ms |
| Convois touchés | 519 | 483 |
| Replanification, temps de calcul | 35 ms | 18 ms |
| Dans la boucle, à 3 ms par tick | 12 ticks | 6 ticks |
| **En tâche de fond**, 4 threads, 15 threads | **10,9 ms, 7,9 ms** | **5,0 ms, 2,4 ms** |
| Les 1 000 convois, en tâche de fond (4 threads) | 15 ms | 8,3 ms |

### 6. Paris

Respecter tous les sens uniques (classe `voiture-code`) change 93 % des chemins, allonge les trajets de 7,5 % en distance et de 11 % en durée en moyenne, et rend 2,8 % des paires impossibles (275 sur 10 000). Dans la Manche : 0,3 % en distance, 0,5 % en durée, 2 paires impossibles. Le Leclerc change 37 % des chemins du camion, pour 3 % de durée en plus ; le porte-char perd 61 % des paires (largeur des rues).

### 7. Déterminisme

Cinq exécutions par jeu de données : 16 threads, 1 thread, ordres de lecture des tiles mélangé et inverse, nouvelle exécution à 16 threads ([determinisme.csv](resultats/determinisme.csv)).

| Empreinte | Manche | Paris |
|---|---|---|
| Fragments | `51cdeedc4b2051b2` | `9200fdd834e45c2a` |
| Graphe | `25299a52ba9c84c6` | `476cf0b1bc41880a` |
| Ordre CCH | `26a8362a4c8eb6eb` | `11e2b2ac35e493c1` |
| Sept métriques | `9ce003757465b0dd` | `cfb64ebe99f881d3` |
| Trajets CCH (2 000 paires) | `68b874349f2f093a` | `ac08c65bfa0af10e` |
| Trajets Dijkstra (500 paires) | `adce02a413f19712` | `6a44732007ecc9af` |

**Identiques** dans les cinq exécutions. Temps en millisecondes entières, égalités départagées par identifiant ; aucune fonction de la bibliothèque mathématique, sauf `sqrt`.

### 8. Suivi en jeu

Camion Cherbourg → Saint-Lô : 307 arcs, 122 tiles traversées.

| Mesure | Résultat |
|---|---|
| Géométrie par les morceaux des tiles (2 445 points) | 0,1 ms |
| Écart des positions locales f32 avec les coordonnées de la chaîne | 0,00 m |
| Position en tile inerte (durées cumulées, recherche dichotomique) | 9 ns |
| Itinéraire restant, brut et compressé | 1 228 et 715 octets |
| 100 modifications du réseau (tronçon BD TOPO, rang de début, état), brut et compressé | 456 et 322 octets |

### 9. Fichier livré

| | Manche | Paris |
|---|---|---|
| Graphe compact, brut et zstd | 2,8 et 2,0 Mo | 0,47 et 0,34 Mo |
| Ordre CCH, brut et zstd | 0,61 et 0,26 Mo | 0,09 et 0,04 Mo |
| Chargement : décompression, structure depuis l'ordre, 4 personnalisations | 3,5 + 56 + 34 ms | 0,8 + 10 + 19 ms |
| Nouvelle traversée (une arête en plus) : structure reconstruite avec le même ordre | 65 ms, 1 095 arcs CCH en plus | |

Micro-banc ([banc.txt](resultats/banc.txt)), grille de 40 000 nœuds : personnalisation complète 157 ms sur un thread (123 ns par arc) ; partielle 194 à 471 ns par arc recalculé.

## Analyse

- **La CCH tient tous les budgets.** Une requête départementale répond en 50 µs en médiane, chemin compris, exactement, quand Dijkstra en prend 7 à 9 ms et ALT 1 à 2 ms. Les CH classiques sont un peu plus rapides en requête (30 µs), mais chaque modification du réseau coûte une nouvelle préparation de 300 ms par classe, contre moins d'une milliseconde pour la CCH. Or le jeu modifie le réseau sans cesse (ponts détruits, gravats, épaves).
- **Analogie** : les CH classiques ressemblent à une table de routage compilée, rapide mais à recompiler entière à chaque changement de lien. La CCH sépare la topologie (les zones et leurs carrefours de frontière, calculés une fois) des coûts, qui se mettent à jour localement, comme un protocole à état de liens qui ne recalcule que les routes touchées.
- **La personnalisation partielle** n'existe pas dans la crate `cch` ; écrite ici, elle est exacte (comparée bit à bit) et 10 à 100 fois plus rapide que la complète pour quelques coupures. Au-delà d'une centaine de coupures simultanées, la complète, parallèle, reprend l'avantage : le jeu peut choisir selon le nombre d'arêtes touchées.
- **Le coût est dans la replanification, pas dans la mise à jour** : 100 coupures se répercutent en 5 ms, mais 500 convois doivent replanifier (35 ms de calcul). En tâche de fond, c'est 8 à 11 ms de temps réel ; dans la boucle, 12 ticks à 3 ms. D'où la tâche de fond, avec un résultat rendu à un tick fixé pour rester déterministe (A-12).
- **Recousure exacte** : la BD TOPO n'a pas de table de nœuds, mais (identifiant, rang) suffit à recoller les morceaux des tiles sans doublon ni rupture, et le graphe ne dépend ni du découpage, ni du nombre de threads, ni de l'ordre de lecture. Un graphe national peut donc se construire département par département.
- **Les ponts** décident des trajets lourds : le Leclerc refuse 39 % des ponts empruntés par les camions. La BD TOPO décrit un pont par tablier : détruire un pont d'une 2 × 2 voies ne coupe qu'un sens, ce qui n'a pas de sens en jeu.
- **Les sens uniques urbains** coûtent cher à Paris (+11 % de durée, 2,8 % de paires impossibles) pour un intérêt de jeu nul dans une France sans circulation.

## Conclusion

L'hypothèse est **vérifiée** : une CCH sur le réseau carrossable recousu depuis les tiles répond en quelques dizaines de microsecondes, exactement, encaisse une destruction en moins d'une milliseconde par classe, et 1 000 convois replanifient en une dizaine de millisecondes en tâche de fond. Le tout est déterministe et tient en moins de 1 Go pour un département.

## Recommandations

- Graphe **national unique**, construit par la chaîne après tous les départements ; ordre CCH calculé par la chaîne, structure et métriques régénérées au chargement du jeu.
- Une **métrique par classe d'itinéraire** (charge, largeur, hauteur, nature des voies), pas une par véhicule.
- Replanification **en tâche de fond**, résultat rendu à un tick fixé ; seuls les convois dont l'itinéraire restant emprunte une arête modifiée replanifient.
- Ajouter aux tiles le lien pont → tronçons portés et le regroupement des tabliers, aujourd'hui retrouvés par la géométrie, et l'attribut « fictif ».

## Décisions

- **CCH** sur un **graphe national unique** : la chaîne calcule l'ordre (dissection emboîtée) et livre un fichier du graphe ; le jeu reconstruit la structure et personnalise une **métrique par classe d'itinéraire** au chargement, en tâche de fond. Une modification du réseau passe par une **personnalisation partielle** ; seuls les convois touchés replanifient, **en tâche de fond**, jamais dans le tick.
- Le **sens de circulation** n'est respecté que sur les **chaussées séparées** (autoroutes, routes à 2 chaussées, bretelles) ; sens uniques urbains, ronds-points et interdictions de tourner ignorés.
- Un **pont du jeu** est une traversée (tabliers regroupés par la chaîne) : détruit, endommagé ou réparé d'un bloc. Gravats, épaves et barrages du génie coupent les arêtes qu'ils recouvrent.
- Une **nouvelle traversée** (pont du génie hors d'un pont existant) ajoute une arête : structure reconstruite avec le même ordre, en tâche de fond (65 ms pour la Manche). Poser un pont du génie sur un pont détruit n'est qu'un changement de métrique.
- La sauvegarde garde la **liste des modifications du réseau** et l'**itinéraire restant** de chaque unité en mouvement ; graphe et métriques se régénèrent.
- La chaîne produit un **fichier national du réseau routier** (graphe carrossable, charge des ouvrages, ordre CCH) ; la section `ponts` des tiles reçoit les tronçons portés, le franchissement et la charge calculée ; la section `reseau` reçoit le bit « fictif ».

Détail et statut : [B-03 et A-07 dans OPEN-PROBLEMS](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md).

## Questions ouvertes

Aucune.
