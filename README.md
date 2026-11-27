# Prototypes

Prototypes techniques jetables. Chaque prototype est une petite codebase indépendante dans son propre sous-dossier (par exemple `p01-tile-paris/`). Il sert à valider un algorithme ou une solution à un problème de [OPEN-PROBLEMS.md](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md), pas à être conservé.

Pour chaque prototype, noter dans son README : l'hypothèse testée, la machine utilisée, les mesures obtenues et la conclusion.

**P-24 à P-26 ne sont pas versionnés** : leurs dossiers restent sur la machine de l'utilisateur (`.gitignore`). Leurs mesures, constats et décisions sont reportés dans [OPEN-PROBLEMS.md](https://github.com/labatailledefrance/documentation/blob/main/OPEN-PROBLEMS.md), qui fait foi.

## Hypothèses communes

- **Machine de mesure** : AMD Ryzen 7 5700X (8 cœurs, 16 threads), 32 Go de DDR4 (2 × 16 Go), NVIDIA RTX 3060 12 Go. Les prototypes P-00 à P-09 y ont été mesurés ; elle reste la machine de mesure des prototypes et la **référence du client** (navigateur).
- **Référence du serveur** (F-02) : `srv.labatailledefrance.fr`, AMD EPYC 7351P (16 cœurs, 32 threads), 128 Go de mémoire, deux disques durs de 8 To en RAID 0, 1 Gbit/s ; k3s et Traefik. Elle fixe le budget d'agents ; les prototypes de capacité (P-14) s'y mesurent.
- **Fréquence de simulation** : hypothèse de départ 20 ticks par seconde, indépendante de la fréquence d'affichage.
- **Budget** (révisé, A-02) : pas d'accélération, simulation sur le serveur. Le tick dispose de **50 ms** à 20 Hz, sans rendu, moins une marge à fixer. Côté client, une frame dure 16,6 ms à 60 images par seconde, pour le rendu seul. Les budgets « à ×10 » et « de ×100 » des prototypes P-01 à P-05 sont historiques.
- Les critères de réussite ci-dessous sont des **premiers objectifs**, à ajuster après les premières mesures.

----

## Phase 1 — Les risques majeurs

Ces tests valident ou invalident l'architecture. À faire en premier.

### P-00 — Socle ECS

- **Problème** : F-01, F-02.
- **Objectif** : mesurer ce qu'un ECS (Bevy en premier) tient sur la machine de référence, avant tout le reste.
- **Méthode** : 1 million d'entités avec position, vitesse et quatre jauges (fatigue, moral, faim, soif) ; mise à jour de toutes les jauges et des positions à chaque tick ; versions mono-cœur et multi-cœur.
- **Critère** : temps par tick pour 100 000, 1 million et 10 millions d'entités. Sert de référence à tous les autres tests.
- **Résultats** : [p00-socle-ecs/](p00-socle-ecs/README.md). Bevy multi-cœur égale du Rust écrit à la main (≈ 2 ms par tick pour 1 million d'entités) ; la limite est la bande passante mémoire (≈ 37 ms à 10 millions, quel que soit le moteur). Les types d'entités variés sont acceptables si l'on fixe une taille de lot minimale sur les requêtes parallèles.

### P-01 — Coût d'une tile parisienne

- **Problème** : A-01, A-15.
- **Objectif** : vérifier qu'une tile active de 1 km² à densité parisienne tient dans le budget.
- **Méthode** : 20 000 zombies agents dans une tile avec des bâtiments (emprises synthétiques d'abord, puis réelles), perception (ouïe, cône de vue), déplacement vers les stimuli, collisions simples.
- **Critère** : nombre de tiles parisiennes actives simulables en même temps dans le budget, à ×1 et à ×10.
- **Résultats** : [p01-tile-paris/](p01-tile-paris/README.md). Sur les emprises réelles de la BD TOPO, avec relais d'alerte et veille, une tile de 20 000 zombies coûte 1,0 à 1,7 ms par tick : environ 12 à 28 tiles actives à ×1 selon le scénario, 1 à 3 à ×10. Le coût est du calcul (pas de la mémoire) et linéaire en nombre de tiles. Le relais gagne 30 à 50 % en combat, la veille 25 % au calme. Le périmètre du périphérique (environ 93 tiles avec zombies) dépassait le budget d'un facteur trois : Paris fortifié a été abandonné au profit de colonies (A-15).

### P-02 — Perception des zombies

- **Problème** : A-01.
- **Objectif** : choisir la structure de données des requêtes de voisinage et de ligne de vue.
- **Méthode** : comparer grille uniforme, grille hiérarchique et arbre (BVH ou k-d tree) pour « quels humains sont dans mon rayon d'ouïe et mon cône de vue, sans bâtiment entre nous ? ».
- **Critère** : requêtes par seconde pour 20 000 zombies et 1 000 humains dans une tile.
- **Résultats** : [p02-perception/](p02-perception/README.md). Sur les tiles réelles de P-01, avec des humains mobiles, un **arbre k-d** par tile, reconstruit à chaque tick (46 µs pour 1 000 humains), divise le coût d'une requête par 8 à 13 (170 à 380 ns, soit 2,6 à 5,8 millions de requêtes par seconde et par thread) et n'a aucun cas pathologique ; les grilles, uniformes, par anneaux ou hiérarchiques, ont toutes un point faible selon la répartition des humains. Le coût ne dépend plus de la distance de vue. Avec le filtre du relais et la veille profonde, environ 44 tiles calmes, 20 dispersées ou 24 en assaut tiennent à ×1 (4, 1 et 2 à ×10), le double de P-01 au calme et en assaut. La perception n'est plus le premier poste : ce sont la grille des zombies, le déplacement et la recherche de relais.

### P-03 — Flow fields

- **Problème** : A-01, A-07.
- **Objectif** : déplacer des hordes entières sans recherche de chemin individuelle.
- **Méthode** : calcul d'un champ de direction sur une tile (résolution à tester : 1 m, 2 m, 5 m) vers un ou plusieurs stimuli, autour des bâtiments ; recalcul quand un obstacle apparaît (barricade, effondrement). Inclut le remplacement de la **grille des zombies** (collisions, relais), premier poste d'une tile calme depuis P-02 : grille creuse, tri des zombies par case, ou cases plus grandes.
- **Critère** : temps de calcul et de recalcul d'une tile ; crédibilité visuelle du mouvement.
- **Résultats** : [p03-flow-fields/](p03-flow-fields/README.md). Sur les tiles réelles, un champ de **2 m** (règle de la majorité, marge le long des murs) rend le mouvement crédible : 0,07 % de zombies bloqués hors foule, course à pleine vitesse, aucun zombie dans un mur, contre 65 % de bloqués en ligne droite. Deux champs par tile à sources multiples (« proies » : humains observés dans la dernière minute ; « bruits »), 2,7 ms de calcul sur un thread, recalculés à **1 Hz en étalant le calcul** sur la seconde (sans étalement, le p99 d'une tile double) ; recalcul partiel exact après un obstacle, 17 fois plus rapide pour une rue barrée, sans gain pour un anneau autour d'une base. Entre tiles, un échange de bordures horodaté converge exactement en 5 recalculs. La **grille dense compacte** (u16, sans curseurs) remplace la grille des zombies : −72 % sur sa phase, −33 % sur le tick calme (moins de pollution du cache) ; grille triée et grille creuse écartées. La **réservation exacte partout** tient la densité à la capacité pour 3 à 4 % du tick (sans elle : jusqu'à 301 zombies sur 4 m²). Au total, environ 40 tiles calmes, 18 à 20 dispersées, 28 en assaut à ×1 ; 2, 1 et 1 à ×10, à la vue de 400 m : mieux que P-02 à la même vue.

### P-04 — Activation et désactivation des tiles

- **Problème** : A-01.
- **Objectif** : transformer « X zombies » en agents et inversement, sans incohérence.
- **Méthode** : placement déterministe des zombies dans les rues (aucun dans les bâtiments à l'activation : B-02) à partir d'une graine ; restauration exacte d'une tile sauvegardée (positions, corps, constructions) ; naissance en veille profonde loin des humains ; délai de désactivation ; désactivation qui sauvegarde chaque zombie (redevenu errant), la carte des corps et les constructions, sur disque ; zombie poursuivant une cible à la frontière d'une tile pendant son activation.
- **Critères** :
  - conservation exacte du nombre de zombies ;
  - même graine, mêmes positions ;
  - activation d'une tile dense étalée sur quelques frames, sans saccade visible.
- **Résultats** : [p04-activation/](p04-activation/README.md). Sur un bloc de 55 tiles réelles de Paris (environ 690 000 zombies), l'activation et la désactivation sont **exactes** : conservation à chaque tick, 366 restaurations identiques octet pour octet (constructions et corps compris), résultat identique de 1 à 16 threads et pour des lots de 64 à 100 000. L'activation **étalée** (zone lue sur disque, terrain par tranches, entités par lots de 2 000, une désactivation par tick) ne dépasse jamais 3 ms par tick ; d'un seul tenant, elle fait monter le tick à 12 à 18 ms. Une colonne dense traverse 10 km à ×1 sans tick au-dessus de 7,3 ms ; à ×10 en banlieue, avec des budgets divisés par 4, le chantier n'ajoute que 0,3 à 0,4 ms au p99. La **zone accessible** doit être précalculée par la chaîne de données : à l'activation, elle coûte 45 à 60 ms et perd jusqu'à 14 % des rues d'une tile. **Sauvegarde** compacte + zstd : environ 4 octets par zombie (51 ko par tile dense), écrite en tâche de fond. Les errants doivent être **renvoyés** au bord d'une tile non active : sinon 10 % de la couronne fuit en 20 min. La **migration** par journal (233 000 migrants en 70 min) a besoin d'une saturation par tile. Trois pièges de l'ECS (journaux de `clear_trackers`, préchauffage des tables, premier champ d'une tile neuve) coûtaient plus que l'activation elle-même.

### P-05 — Civils agrégés

- **Problème** : A-15.
- **Objectif** : simuler 1 million de colons, répartis dans des colonies, sans 1 million d'agents complets. Les colonies grossissent au fil de la partie (jusqu'à plusieurs millions de colons disponibles en outre-mer).
- **Méthode** : civils agrégés par bâtiment ou îlot (effectifs, besoins moyens, réserves) ; individualisation quand la caméra s'approche ou qu'un contact avec des zombies se produit ; retour à l'agrégat ensuite.
- **Critères** : coût par tick pour 1 million de civils ; aucune perte ni duplication lors des transitions.
- **Résultats** : [p05-civils/](p05-civils/README.md). Sur les 606 000 bâtiments réels de la BD TOPO, l'agrégat statistique n'est **pas nécessaire** : des **dossiers compacts par bâtiment** (32 octets par colon, hors de l'ECS) font exactement le même calcul qu'une entité par colon, vérifié par somme de contrôle. Avec des jauges toutes les 10 s, 1 million de colons coûtent **0,08 ms par tick** (un tiers du budget de ×100), 3 millions 0,15 ms. Une entité par colon coûte 2,5 à 12 fois plus selon le rangement, et 3 fois plus de mémoire. À 1 Hz, les civils prendraient à eux seuls tout le budget de ×100. L'agrégat casse A-18 et compte 63 % de morts de plus en hiver. Individualiser puis rendre à l'agrégat est exact (état identique à une simulation sans caméra) et coûte 80 à 290 ns par colon ; un bâtiment s'individualise en 1 tick. Mais les occupants d'un bâtiment n'ont rien à gagner à être des entités : seuls les colons dehors en ont besoin. Un **point pondéré par bâtiment** suffit à la perception des zombies, pour 10 fois moins qu'un point par occupant. Pendant un assaut, la chute d'une colonie émerge de l'auto-résolution (571 bâtiments sur 574 tombent sans arme ; avec 5 % de miliciens, la colonie perd 29 bâtiments puis tient), avec une conservation exacte et une entité zombie par zombifié. Sauvegarde : 16 octets par colon. Poste à revoir : un chemin à pied sous la caméra coûte jusqu'à 10 ms d'un tenant. Décisions prises : dossiers par bâtiment, individualisation des seuls colons dehors, jauges toutes les 10 s, un point par porte, A\* reprenable ([décisions](p05-civils/README.md#décisions)).

### P-06 — Origine flottante et précision

- **Problème** : C-01.
- **Objectif** : vérifier la précision au centimètre partout, de Brest à Tahiti.
- **Méthode** : simulation en coordonnées 64 bits, rendu relatif à la caméra ; caméra posée sur un soldat à Brest, puis à Tahiti, puis au point le plus éloigné de l'origine.
- **Critère** : aucun tremblement visible des modèles ni de la caméra.
- **Résultats** : [p06-origine-flottante/](p06-origine-flottante/README.md). Le rendu relatif à la caméra résout C-01 partout : écart sous 0,02 px (0,1 mm) à Brest, à Tahiti et au point le plus éloigné de Paris (l'île Walpole, en Nouvelle-Calédonie, à 16 866 km), confirmé par des captures du rendu réel ; les témoins sans origine flottante tremblent de 8 px (Brest) à 300 px (Walpole). Pour la simulation, des coordonnées **tile + position locale f32** (8 octets) sont les moins chères et les seules invariantes par translation ; l'ECEF en 64 bits coûte 20 fois plus par tick (projection à chaque pas), la grille en 64 bits double la mémoire. Le repère d'une tile doit être au **second ordre** (le plan tangent est faux de 39 mm aux coins) ; les navires changent de zone par l'ECEF sans aucun saut. Le recalage **au-delà d'un seuil** de quelques kilomètres coûte 0,25 ms à 100 000 modèles, contre 2,3 ms (et 8,6 ms de frame) à chaque frame, qui fait de plus trembler les grands maillages ; `big_space` n'est pas mûr (version candidate, remplace la propagation de Bevy) ; la hiérarchie de Bevy repropage tous les frères d'un enfant qui bouge. Profondeur sans problème (écart net de 3 × 10⁻⁷ fois la distance). Le temps des shaders en f32 absolu devient visible après 36 h de jeu : phases calculées depuis les ticks entiers. Décisions prises : tile + position locale f32 centrée, ECEF en mer, repère de tile au second ordre, recalage au-delà de 5 km, ticks u64 et phases pour les shaders, instanciation des foules, pas de TAAF ([décisions](p06-origine-flottante/README.md#décisions)).

### P-22 — Réseau et gestion d'intérêt

- **Problème** : H-05, H-06, A-12, C-05.
- **Objectif** : tenir **1 000 joueurs simultanés** sur 1 Gbit/s montant, tiles comprises, sans CDN.
- **Méthode** : serveur qui fait autorité (celui de P-24, avec ses clients simulés) et clients simulés, chacun avec une caméra en mouvement (survol, suivi d'un combat en ville dense, vue simplifiée). Gestion d'intérêt par zone : zombies un par un jusqu'à environ 1 km de la caméra, carte de densité par case de 50 m au-delà (décision de P-24), positions quantifiées en différences ; tiles servies à la demande, en HTTP statique, avec cache côté client.
- **Critères** : débit montant moyen et p99 par client (objectif de P-24 : 60 ko/s) ; coût de la gestion d'intérêt dans le tick du serveur ; délai entre un ordre et son effet à l'écran.

### P-23 — Client navigateur

- **Problème** : F-01, F-04, C-04, C-05.
- **Objectif** : afficher une tile dense de Paris dans le navigateur, avec Bevy compilé en WebAssembly et rendu en WebGPU, sur la machine de mesure (Linux, Wayland).
- **Méthode** : partir du client natif de P-24 ; bâtiments générés dans le client (P-08), étalés sur plusieurs frames ou confiés à des Web Workers ; zombies reçus du serveur et rendus par instanciation ; mémoire suivie sous la limite de 4 Go du wasm32. Vérifier d'abord la disponibilité de WebGPU sous Linux dans Chrome et Firefox.
- **Critères** : p99 du temps de frame ; temps entre l'arrivée d'une tile et son affichage ; mémoire maximale du client.

## Phase 2 — Le monde

### P-07 — Chaîne de données IGN sur la Manche

- **Problème** : B-01.
- **Objectif** : produire les tiles de jeu d'un département à partir des données brutes.
- **Méthode** : BD TOPO et RGE ALTI de la Manche → tiles de 1 km (relief, bâtiments, routes, ponts, végétation, hydrographie).
- **Critères** : temps de traitement, taille des tiles générées, extrapolation à la France entière.
- **Résultats** : [p07-chaine-manche/](p07-chaine-manche/README.md). La Manche entière (14 076 tiles, dont 7 808 de mer à 30 km) se traite en 66 s sur la machine de référence, avec 11 Go de mémoire ; le relief est lu en flux dans l'archive 7z (6 701 dalles, 43 Go d'ASC, aucune place disque temporaire). Une tile fait 60 ko en rural, 120 ko en ville, 210 ko en centre dense ; le **relief** (nœuds de 2 m quantifiés à 10 cm, 3 cm d'erreur RMS) en fait 80 % en rural. Mêmes octets de 1 à 16 threads ; aucune marche sur 6,3 millions de nœuds de bord, aucun bâtiment en double, population conservée exactement. Chargement d'une tile : 3 à 8 ms sur un thread, à étaler. Extrapolation à la France (hors Pacifique) : 800 000 tiles, 35 à 41 Go, 1,9 h de calcul ; à l'installation, il faudrait télécharger 594 Go de sources (RGE ALTI 1 m : 561 Go) : **traitement avant distribution** retenu, tiles téléchargées en entier à l'installation. Constats : carreaux INSEE en EPSG 3035 (pas en Lambert-93), tiles parisiennes jusqu'à 42 600 habitants, restriction de poids sur 1 pont sur 3 292, l'IGN ne couvre pas le Pacifique (retiré du jeu, SNLE basés à Saint-Pierre-et-Miquelon).

### P-08 — Génération des bâtiments

- **Problème** : B-02.
- **Objectif** : générer chaque bâtiment à partir de son emprise et de sa hauteur, assez vite pour le streaming.
- **Méthode** : génération déterministe d'un modèle 3D par bâtiment à partir d'une graine ; mesure du débit.
- **Critères** : bâtiments générés par seconde ; même graine, même bâtiment ; temps de génération d'une tile parisienne.
- **Résultats** : [p08-generation-batiments/](p08-generation-batiments/README.md). Sur les tiles de P-07 (720 092 bâtiments de la Manche et de Paris), le modèle (emprise réelle extrudée, soubassement borné, repli local de la hauteur, toit en fonction affine par morceaux choisi d'après la hauteur du toit de la BD TOPO, fenêtres dans la texture, porches par opération booléenne) n'a **aucun échec** : 7 × 10⁻⁵ des cases de bâti sans volume, aucun doublon. Génération : 9 à 16 µs par bâtiment, **0,6 ms par tile rurale, 16 à 80 ms par tile urbaine ou dense** sur un thread, 0,2 h pour la France ; étalée, jamais plus de 3,08 ms par tick. **Stocker** les maillages coûterait 28 à 45 Go (20 fois la section des bâtiments) : on **génère** au chargement, en tâche de fond. La **porte** sur la case accessible au pied des murs la plus proche de la rue donne sur la zone pour 97,6 à 99,6 % des bâtiments, deux fois plus près de la rue que la règle de P-05 (qui en met une sur quinze hors du bâtiment à Paris) ; elle demande la zone et les voisines : calcul de chaîne. Rendu : 49 tiles parisiennes (12,4 millions de triangles) en 3 à 4 ms ; les à-coups du streaming viennent de l'**envoi** d'une tile fusionnée de 15 Mo d'un bloc (Bevy recopie tous les maillages de la frame) : des **morceaux de 250 m** envoyés par une file de 2 Mo par frame les suppriment (0 frame au-dessus du budget à basse altitude, aucune tile manquante à 1 km/s). Une entité par bâtiment fait 76 000 entités et des pics de 48 ms à 49 tiles. Mêmes octets sur 1 à 16 threads et dans tout ordre, sans bibliothèque mathématique. Constats : hauteur du toit présente pour 76 % des bâtiments de la Manche mais absente des tiles, emprises auto-sécantes, nature « Monument » inutilisable (statues), gravats trop lourds en blocs de 2 m. Décisions prises : modèle de P-08, porte calculée par la chaîne, bâtiments générés au chargement et jamais stockés, rendu en morceaux de 250 m envoyés par une file, monuments sur liste écrite à la main, attributs ajoutés au format de tile ([décisions](p08-generation-batiments/README.md#décisions)).

### P-09 — Recherche de chemin routière

- **Problème** : A-07, B-03.
- **Objectif** : itinéraires de véhicules sur le réseau routier réel, avec charge limite des ponts.
- **Méthode** : réseau routier de la Manche (BD TOPO), recherche hiérarchique (contraction hierarchies ou équivalent) ; contrainte de poids par véhicule (Leclerc contre camion) ; mise à jour quand un pont est détruit ou une route bloquée.
- **Critères** : temps de requête, temps de mise à jour après modification du réseau.

### P-10 — Streaming et niveaux de vue

- **Problème** : C-02, C-03, C-04.
- **Objectif** : zoom continu du soldat en vue isométrique à la carte 2D, puis au globe, et trajet Paris → La Réunion sans écran de chargement.
- **Méthode** : chargement et déchargement des tiles 3D et 2D à la volée pendant les mouvements de caméra, **dans le navigateur**, les tiles venant du serveur (P-22, P-23). Les bâtiments y sont générés en tâche de fond et envoyés par morceaux de 250 m (P-08).
- **Critère** : aucune frame au-dessus du budget (mesure du p99 du temps de frame) pendant le trajet.

### P-11 — Rendu des foules

- **Problème** : C-05.
- **Objectif** : afficher des milliers de zombies en vue isométrique.
- **Méthode** : instanciation GPU, animation sur GPU, imposteurs à distance ; carte de densité en vue 2D.
- **Critère** : nombre de zombies affichables à 60 images par seconde.

### P-12 — Lisibilité en ville

- **Problème** : C-06.
- **Objectif** : voir les unités à travers les bâtiments parisiens en vue isométrique.
- **Méthode** : comparer transparence, découpe autour des unités et silhouettes à travers les murs. Les bâtiments sont des morceaux fusionnés de 250 m dont chaque sommet porte l'indice de son bâtiment (P-08) : la transparence et la découpe se font dans le shader.
- **Critère** : lisibilité jugée en test de jeu ; coût de rendu.

## Phase 3 — Les systèmes

### P-13 — Migration entre tiles inertes

- **Problème** : A-01.
- **Objectif** : faire bouger les hordes dans toute la France pour un coût négligeable.
- **Méthode** : environ 550 000 tiles inertes avec effectifs initiaux INSEE ; flux entre tiles voisines attirés par les stimuli (bruit, humains).
- **Critères** : coût par tick (à ×1, A-02) ; conservation exacte des effectifs.

### P-14 — Capacité du serveur

- **Problème** : A-02, A-03, F-02. (Ce prototype mesurait l'accélération maximale tenable ; l'accélération est abandonnée.)
- **Objectif** : trouver le nombre de tiles actives tenables à ×1 sur le serveur, toutes les unités du joueur réellement simulées (A-03).
- **Méthode** : sur `srv.labatailledefrance.fr`, tiles calmes, en assaut et dispersées (P-01 à P-04) ; mise en veille des unités sans stimulus ; fréquence adaptative des zones calmes ; ralentissement sous ×1 quand le budget est dépassé.
- **Critères** : tiles actives tenables à 20 Hz, par type de tile ; durée et fréquence des ralentissements sous ×1.

### P-15 — Combat probabiliste et explosions

- **Problème** : A-04.
- **Objectif** : résoudre des milliers de tirs par seconde.
- **Méthode** : chance de toucher la tête selon l'arme, la distance et l'entraînement ; explosions par zone.
- **Critère** : tirs résolus par seconde ; cohérence statistique avec les probabilités attendues.

### P-16 — Entassement et franchissement

- **Problème** : A-05.
- **Objectif** : rendre crédible le franchissement d'obstacles par entassement.
- **Méthode** : seuil de densité contre un obstacle → rampe virtuelle ; dégâts aux structures selon la même logique.
- **Critères** : crédibilité visuelle ; coût par obstacle.

### P-17 — Feu

- **Problème** : A-06, B-04.
- **Objectif** : simuler un incendie de 50 km² arbre par arbre.
- **Méthode** : automate cellulaire et objets dans les zones en feu, état stocké ailleurs ; vent ; agrégat dans les tiles inertes ; zombies en feu qui propagent l'incendie.
- **Critère** : coût par tick d'un incendie de 50 km².

### P-18 — Déterminisme

- **Problème** : A-12, A-13.
- **Objectif** : vérifier que la simulation est reproductible.
- **Méthode** : mêmes entrées → empreinte (hash) de l'état identique après N ticks ; entre deux exécutions, avec un nombre de threads différent. (Le serveur faisant autorité, le multijoueur n'en dépend plus : priorité basse, utile au débogage, A-12.)
- **Critère** : empreintes identiques.

### P-19 — Sauvegarde différentielle

- **Problème** : A-13.
- **Objectif** : instantanés du monde ne contenant que ce qui diffère du monde régénéré, pris par le serveur sans arrêter la partie (A-13).
- **Méthode** : partie simulée de plusieurs heures (destructions, constructions, objets déplacés) ; instantané pendant la simulation, arrêt brutal, reprise.
- **Critères** : taille de l'instantané, durée et effet sur le tick (un toutes les 5 minutes, sur les disques durs chiffrés du serveur), temps de reprise.

### P-20 — Cargaison et emballage

- **Problème** : A-09, D-05.
- **Objectif** : valider le modèle de données des objets (lots, emballer / déballer, inspection).
- **Méthode** : conteneurs contenant des palettes contenant des caisses contenant des lots ; calcul de masse et de volume ; inspection.
- **Critère** : temps de calcul de la masse d'un navire chargé ; simplicité du modèle.

### P-21 — Marées

- **Problème** : A-11.
- **Objectif** : calculer la marée sur les plages de Normandie.
- **Méthode** : prédiction par harmoniques, comparée à des données de référence (pour la mesure seulement : les prédictions du SHOM ne peuvent pas entrer dans le jeu, B-06).
- **Critère** : écart avec la référence.

## Phase 4 — Intégration

### P-24 — Prototype jouable local

- **Problème** : A-01, A-02, A-04, A-07, A-12, A-16, F-01, F-04, H-03, H-05.
- **Objectif** : assembler les briques de P-00 à P-09 en une partie jouable sur une zone réelle, avec l'architecture décidée : serveur sans rendu qui fait autorité, client qui affiche et commande sans simuler.
- **Méthode** : 6 × 6 km de la Manche (Utah Beach, Sainte-Marie-du-Mont) depuis les tiles de P-07 ; serveur Bevy à 20 Hz et client Bevy natif reliés par WebSocket ; zombies (perception, relais, flow fields), combat infanterie et Griffon, véhicules sur le graphe routier, dépôt, camions, ramassage des corps et site d'incinération dont la fumée attire les hordes.
- **Critères** : jouable de bout en bout ; coût du tick (p50, p99) ; débit par client ; temps de frame du client ; déterminisme.
- **Résultats** (dossier local `p24-jouable/`, non versionné) : Jouable de bout en bout : débarquement, avance, combat, ravitaillement par camions, ramassage et incinération des corps, hordes attirées par les tirs et la fumée. 27 000 zombies sur 36 km² coûtent **2,3 ms par tick** (p99 2,4 ms), dont 1 ms de déplacement ; les champs (20 à 30 ms, hors du tick) sont le premier poste. Un client qui voit 1,5 km reçoit **32 ko/s** (p99 40 ko/s) ; qui voit toute la zone, 475 ko/s, au-delà du budget de H-05 : carte de densité nécessaire au loin. Client : 4,6 millions de triangles et 5 100 zombies, frame p99 12 ms. **Déterminisme** obtenu en appliquant les résultats des tâches de fond à un tick fixé (sans cela, trois exécutions, trois empreintes). Décisions prises : tâches de fond à tick fixé, `game/` sur l'architecture de P-24, densité au-delà de 1 km, champs limités aux tiles actives, foule mêlant pente et écart individuel, ordres logistiques permanents, gué des cours d'eau étroits. Second temps : siège du Mont-Saint-Michel avec modèles, textures, foule animée et passe pixel art.

### P-25 — Manche jouable

- **Problème** : A-01, A-03, A-07, C-01 à C-05, E-01, H-05, H-06.
- **Objectif** : jouer dans la **Manche entière** (environ 6 300 tiles de terre) avec le rendu de P-24 : interface utilisateur, zoom continu du soldat à la carte du département, tiles chargées et activées à la demande.
- **Méthode** : reprendre P-24 ; positions en tile + position locale (C-01, P-06), streaming et niveaux de vue du client (P-10, en natif et sans le globe), tiles actives et inertes (P-04), routes en CCH (P-09), zombies en différences et carte de densité au loin (P-22). Deux foyers de jeu, Utah Beach et le Mont-Saint-Michel, dans la même partie.
- **Critères** : p99 du temps de frame pendant un survol et un zoom complets ; mémoire du client et du serveur, bornées et indépendantes de la taille de la carte ; tick p99 avec plusieurs combats simultanés ; débit par client (60 ko/s visés) ; déterminisme ; jouable de bout en bout à la souris.
- **Prompt de départ** : `p25-manche/PROMPT.md` (local).
- **Résultats** (dossier local `p25-manche/`, non versionné) : La Manche se joue du soldat au département. Tick p99 **7,6 ms** (deux combats de 10 000), débit p99 **30 ko/s** au-dessus d'un combat, comptes exacts, même empreinte à 1 et 16 threads, interface testée à la souris. Manqués : mémoire du client (3,8 Go, en hausse), une frame de 46,8 ms sur un tour, siège « tenable » non démontré.

### P-26 — France entière

- **Problème** : A-01, A-10, B-01, B-03, C-01 à C-05, F-02, G-02, H-05, H-06.
- **Objectif** : le jeu de P-25 sur la métropole et l'outre-mer, avec le relief le plus fidèle possible ; données produites et gardées sur `srv.labatailledefrance.fr`, tiles téléchargées à la demande par le client.
- **Méthode** : chaîne de P-07 sur tous les départements, sur le serveur ; jeu de P-25 sans tableaux fixes, tile = (zone de projection, ix, iy) ; chargeur HTTP avec cache ; graphe routier national ; foyers de Paris et de La Réunion en plus de ceux de P-25.
- **Critères** : couverture contrôlée ; frame p99 sur un survol du pays ; délai d'arrivée d'une tile ; mémoire indépendante de la carte ; tick avec un foyer à Paris ; débit ; comptes et déterminisme.
- **Prompt de départ** : `p26-france/PROMPT.md` (local).
- **État** (2026-11-25, en cours ; dossier local `p26-france/`, non versionné) : jeu sans tableaux fixes (tile = zone, ix, iy) ; chaîne généralisée, conforme à P-07 sur la Manche et Paris, produite aussi pour La Réunion et les Hautes-Alpes ; tiles et partie servies depuis le cluster et **jouables** par tunnel (Manche, La Réunion) ; globe continu et carte multi-zones. Relief national retenu : LiDAR HD en 1 m, complété par le RGE ALTI. Restent : téléchargements (RGE ALTI, LiDAR HD), reste de la France, graphe routier national, mesures complètes, étapes 6 et 7. Détail dans OPEN-PROBLEMS (B-01, H-06, C-01 à C-04, A-07, H-01).
