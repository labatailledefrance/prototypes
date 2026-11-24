# P-09 — Recherche de chemin routière

Prototype jetable. Problèmes : [A-07](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-07--recherche-de-chemin), [B-03](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#b-03--charge-limite-des-ponts), [A-02](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-02--accélération-du-temps), [A-03](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-03--unités-dispersées-sur-tout-le-territoire), [A-12](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-12--déterminisme-et-multijoueur), [A-13](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#a-13--sauvegardes), [B-01](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#b-01--chaîne-de-traitement-des-données-ign), [B-05](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#b-05--destructions-et-modifications-du-terrain), [F-02](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation). Fiche du prototype : [prototypes/README.md](../README.md#p-09--recherche-de-chemin-routière). Premier prototype sur le **réseau routier** des tiles de [P-07](../p07-chaine-manche/README.md).

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
