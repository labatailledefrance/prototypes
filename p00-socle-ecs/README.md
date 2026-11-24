# P-00 — Socle ECS

Prototype jetable. Problèmes : [F-01](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-01--moteur), [F-02](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation). Fiche du prototype : [prototypes/README.md](../README.md#p-00--socle-ecs).

## Hypothèse testée

Un ECS (Bevy en premier) tient-il 100 000, 1 million et 10 millions d'entités sur la machine de référence, avec position, vitesse et quatre jauges (fatigue, moral, faim, soif) mises à jour à chaque tick ? Combien coûte l'ECS par rapport à du code Rust écrit à la main ?

## Machine

Machine de référence (F-02) : AMD Ryzen 7 5700X (8 cœurs, 16 threads, 32 Mo de cache L3), 32 Go de DDR4 (fréquence non relevée, à vérifier), Linux 7.0. Rust 1.99, `bevy_ecs` 0.20.1, `rayon` 1.12. Mesures du 2026-11-14.

## Méthode

Trois implémentations font **exactement le même calcul** (fonctions communes dans [src/logique.rs](src/logique.rs)) :

- **Rust brut** ([src/brut_bench.rs](src/brut_bench.rs)) : un tableau par champ, boucles simples ou `rayon`. C'est le plafond : ce qu'on peut faire de mieux à la main.
- **Bevy** ([src/bevy_bench.rs](src/bevy_bench.rs)) : `bevy_ecs` seul, sans fenêtre ni rendu ; deux systèmes (déplacement, jauges), exécuteur mono-thread + `iter_mut`, ou exécuteur multi-thread + `par_iter_mut`.
- **Bevy sans détection** : comme Bevy, mais les écritures contournent la *détection de changement* (voir plus bas).

Données par entité : position en 3 × `f64` (coordonnées 64 bits, cf. C-01), vitesse en 3 × `f32`, quatre jauges en `f32`. Un quart des entités est immobile. Deux dispositions des jauges : **séparées** (un composant par jauge) ou **groupées** (un seul composant `Jauges`).

Tick : `position += vitesse × dt`, puis faim et soif qui montent, fatigue qui monte en mouvement et descend au repos, moral qui dépend des trois autres. Les deux systèmes ne se gênent pas et tournent en même temps en mode multi-thread.

Mesure : 20 ticks de préchauffage, puis 200 ticks chronométrés (100 à 10 millions) ; médiane et p99. Compilation `release`, LTO complète. Un test (`cargo test --release`) vérifie que les 12 variantes donnent la même somme de contrôle ; c'est aussi vérifié sur chaque ligne des résultats.

Reproduire : `./run.sh` (écrit [resultats.csv](resultats.csv)) ; `NATIVE=1 ./run.sh` pour `target-cpu=native` ([resultats-native.csv](resultats-native.csv)) ; `./run-archetypes.sh` pour le test 2 ([resultats-archetypes.csv](resultats-archetypes.csv)) ; `./run-etats.sh` pour le test 3 ([resultats-etats.csv](resultats-etats.csv)).

## Budgets de référence

Hypothèses de [prototypes/README.md](../README.md) : la simulation dispose de la moitié d'une frame à 60 images/s, et tourne à 20 ticks/s.

| Accélération | Budget par tick |
|---|---|
| ×1 | 25 ms |
| ×10 | 2,5 ms |
| ×100 | 0,25 ms |

## Mesures

Temps par tick en ms : **médiane (p99)**. Compilation portable.

#### 100 000 entités

| Jauges | Mode | Rust brut | Bevy | Bevy sans détection |
|---|---|---|---|---|
| séparées | mono-cœur | 0,321 (0,386) | 1,27 (1,31) | 0,807 (1,31) |
| séparées | 4 threads | 0,102 (0,115) | 0,278 (0,361) | 0,119 (0,156) |
| séparées | 16 threads | 0,058 (0,161) | 0,166 (0,201) | 0,099 (0,127) |
| groupées | mono-cœur | 0,448 (0,512) | 0,652 (0,698) | 0,631 (0,695) |
| groupées | 4 threads | 0,138 (0,151) | 0,182 (0,223) | 0,179 (0,243) |
| groupées | 16 threads | 0,078 (0,131) | 0,118 (0,140) | 0,117 (0,138) |

#### 1 000 000 entités

| Jauges | Mode | Rust brut | Bevy | Bevy sans détection |
|---|---|---|---|---|
| séparées | mono-cœur | 3,97 (5,67) | 13,88 (15,32) | 8,71 (9,32) |
| séparées | 4 threads | 1,96 (2,84) | 3,37 (4,20) | 1,86 (2,49) |
| séparées | 16 threads | 2,44 (3,46) | 3,73 (4,85) | 1,84 (2,32) |
| groupées | mono-cœur | 5,02 (5,63) | 7,37 (8,06) | 6,97 (7,54) |
| groupées | 4 threads | 1,91 (2,37) | 2,38 (3,14) | 1,87 (2,13) |
| groupées | 16 threads | 1,90 (2,89) | 2,81 (3,26) | 1,81 (2,02) |

#### 10 000 000 entités

| Jauges | Mode | Rust brut | Bevy | Bevy sans détection |
|---|---|---|---|---|
| séparées | mono-cœur | 45,71 (48,18) | 156,12 (157,79) | 101,87 (105,37) |
| séparées | 4 threads | 37,15 (39,07) | 53,42 (55,16) | 37,07 (39,75) |
| séparées | 16 threads | 38,94 (40,69) | 54,59 (56,85) | 38,99 (41,45) |
| groupées | mono-cœur | 57,38 (61,13) | 81,67 (82,77) | 77,40 (79,14) |
| groupées | 4 threads | 36,77 (38,72) | 42,97 (45,17) | 36,58 (38,35) |
| groupées | 16 threads | 38,55 (40,05) | 45,16 (46,56) | 38,81 (40,31) |

Les mesures à 1, 2 et 8 threads sont dans le CSV.

#### Mémoire et création

| | 100 000 | 1 million | 10 millions | Par entité |
|---|---|---|---|---|
| Rust brut | 8 Mo | 53 Mo | 499 Mo | ≈ 50 octets |
| Bevy, jauges séparées | 19 Mo | 150 Mo | 1 608 Mo | ≈ 160 octets |
| Bevy, jauges groupées | 17 Mo | 127 Mo | 1 379 Mo | ≈ 140 octets |

Création des entités : Rust brut ≈ 0,05 µs par entité ; Bevy ≈ 0,12 à 0,15 µs (1,2 à 1,5 s pour 10 millions, ≈ 3 ms pour 20 000 zombies).

#### `target-cpu=native`

Gain de 10 à 25 % seulement quand le calcul domine (petits effectifs, mono-cœur) ; aucun gain au-delà de 1 million, où la mémoire domine. Ne justifie pas d'abandonner un binaire portable.

## Analyse

**1. Le mur, c'est la mémoire, pas le processeur.** Au-delà d'environ 1 million d'entités, les données ne tiennent plus dans le cache L3 (32 Mo) et toutes les implémentations plafonnent au même niveau : ≈ 37 ms à 10 millions, dès 2 à 4 threads. Ajouter des cœurs ne sert plus à rien. Analogie : un NAS dont le lien 10 GbE est saturé ; ajouter des CPU au serveur ne débite pas plus vite. À 100 000 entités (5 Mo, tient dans le cache), le passage à l'échelle est bon jusqu'à 8 threads.

**2. 10 millions d'agents complets à chaque tick, c'est impossible, quel que soit le moteur.** Même le Rust brut (37 ms) dépasse le budget de ×1 (25 ms). Cela confirme la simulation par tiles (A-01) : seuls les agents des tiles actives sont des entités complètes. À 1 million, on tient ×1 largement et ×10 de justesse (≈ 1,8 à 2,4 ms pour 2,5 ms).

**3. Bevy multi-cœur vaut le Rust brut, à la détection de changement près.** Bevy horodate chaque écriture de composant pour permettre aux systèmes de demander « qu'est-ce qui a changé depuis mon dernier passage ? » (analogie : un `auditd` qui journalise chaque écriture). Ce suivi coûte 30 à 45 % du temps de tick et une bonne part de la mémoire supplémentaire. Sans lui, Bevy multi-cœur égale le Rust brut. On peut le contourner système par système (`bypass_change_detection`) là où il ne sert à rien, comme les jauges qui changent à chaque tick.

**4. Bevy en mono-cœur est 2 à 3,5 fois plus lent que le Rust brut.** Sans conséquence : le jeu sera multi-cœur. Note : en mode multi-thread, le thread principal participe au travail, d'où « Bevy, 1 thread » plus rapide que « Bevy, mono-cœur ».

**5. Avec Bevy, grouper les données mises à jour ensemble.** Jauges groupées : 2,4 ms contre 3,4 ms à 1 million (moins de colonnes et d'horodatages à écrire), et 15 % de mémoire en moins. Le Rust brut préfère au contraire les tableaux séparés (mieux vectorisés), mais l'écart disparaît en multi-cœur.

**6. Coût fixe de l'exécuteur Bevy : ≈ 0,05 à 0,1 ms par tick** pour deux systèmes (visible à 100 000 entités). Négligeable à ×1, sensible à ×100 (budget de 0,25 ms) : à remesurer avec un vrai nombre de systèmes (P-14).

**7. La mémoire n'est pas un problème** : 10 millions d'entités Bevy tiennent en 1,6 Go sur 32 Go.

## Test 2 — Plusieurs types d'entités (archétypes)

### Question

Dans le jeu, les entités n'auront pas toutes les mêmes composants : soldats, civils, zombies, véhicules, et des états (blessé, en feu…). Bevy range chaque combinaison distincte de composants dans sa propre table, un **archétype**. Analogie : une base qui créerait une table par combinaison de colonnes non vides. Plus il y a de types différents, plus les tables sont nombreuses et petites. Combien cela coûte-t-il ?

### Méthode

Mêmes entités et mêmes systèmes que le test 1 (Bevy, jauges groupées, détection de changement active), mais réparties entre 1 et 1 024 archétypes grâce à dix composants marqueurs sans données (2¹⁰ combinaisons). Dans un stockage en colonnes, une colonne qu'une requête ne lit pas ne coûte rien : seuls comptent le nombre de tables et leur taille. Les marqueurs simulent donc fidèlement des types d'entités variés. Effectifs : 20 000 (une tile parisienne, cf. A-01), 100 000 et 1 million.

Deux découpages parallèles comparés : celui de Bevy par défaut, et une **taille de lot minimale** de 4 096 entités (`BatchingStrategy::min_batch_size`).

### Mesures

Temps par tick en ms : **médiane (p99)**.

#### 20 000 entités

| Archétypes | Mono | 4 threads | 4 threads, lot ≥ 4 096 | 16 threads | 16 threads, lot ≥ 4 096 |
|---|---|---|---|---|---|
| 1 | 0,131 (0,158) | 0,053 (0,083) | 0,053 (0,069) | 0,050 (0,063) | 0,052 (0,073) |
| 4 | 0,129 (0,138) | 0,051 (0,077) | 0,052 (0,061) | 0,082 (0,111) | 0,048 (0,090) |
| 16 | 0,133 (0,155) | 0,071 (0,122) | 0,054 (0,083) | 0,261 (0,440) | 0,050 (0,063) |
| 64 | 0,138 (0,158) | 0,218 (0,254) | 0,055 (0,065) | 1,11 (1,39) | 0,056 (0,074) |
| 256 | 0,167 (0,216) | 0,864 (0,920) | 0,066 (0,085) | 4,42 (4,95) | 0,063 (0,086) |
| 1 024 | 0,283 (0,294) | 3,39 (3,54) | 0,097 (0,138) | 11,16 (12,17) | 0,085 (0,109) |

#### 100 000 entités

| Archétypes | Mono | 4 threads | 4 threads, lot ≥ 4 096 | 16 threads | 16 threads, lot ≥ 4 096 |
|---|---|---|---|---|---|
| 1 | 0,652 (0,673) | 0,184 (0,229) | 0,181 (0,203) | 0,120 (0,189) | 0,119 (0,178) |
| 4 | 0,632 (0,653) | 0,144 (0,162) | 0,146 (0,198) | 0,121 (0,162) | 0,118 (0,203) |
| 16 | 0,651 (0,760) | 0,155 (0,197) | 0,151 (0,172) | 0,247 (0,434) | 0,119 (0,156) |
| 64 | 0,660 (0,689) | 0,238 (0,296) | 0,151 (0,174) | 1,13 (1,36) | 0,119 (0,191) |
| 256 | 0,701 (0,736) | 0,870 (1,15) | 0,162 (0,180) | 4,46 (4,86) | 0,125 (0,199) |
| 1 024 | 0,856 (0,904) | 3,35 (3,50) | 0,205 (0,246) | 15,46 (17,29) | 0,141 (0,466) |

#### 1 million d'entités

| Archétypes | Mono | 4 threads | 4 threads, lot ≥ 4 096 | 16 threads | 16 threads, lot ≥ 4 096 |
|---|---|---|---|---|---|
| 1 | 7,37 (7,82) | 2,85 (3,36) | 2,92 (3,38) | 2,95 (3,74) | 2,92 (3,28) |
| 4 | 7,41 (7,75) | 2,93 (3,46) | 2,85 (3,18) | 2,95 (3,41) | 2,91 (3,49) |
| 16 | 7,54 (8,08) | 2,85 (3,51) | 2,89 (3,35) | 3,03 (3,61) | 2,97 (3,55) |
| 64 | 7,52 (7,91) | 2,91 (3,34) | 2,90 (3,74) | 3,17 (3,82) | 3,00 (3,61) |
| 256 | 7,60 (7,96) | 3,26 (3,79) | 3,01 (3,40) | 4,94 (5,66) | 3,14 (3,85) |
| 1 024 | 8,45 (8,88) | 4,19 (4,89) | 3,49 (3,89) | 18,12 (19,44) | 3,52 (4,07) |

Toutes les variantes donnent la même somme de contrôle que le test 1.

### Analyse

**1. En mono-cœur, la fragmentation est presque gratuite** : +15 % à 1 million d'entités avec 1 024 archétypes.

**2. En parallèle, avec le découpage par défaut, elle est catastrophique.** Sur une tile parisienne (20 000 entités), 1 024 archétypes et 16 threads font passer le tick de 0,05 ms à 11 ms, soit ×200. Le surcoût ne dépend pas du nombre d'entités, il est proportionnel à archétypes × threads (≈ 0,35 µs chacun, par système). Cause, lue dans le code de Bevy (`QueryParIter::get_batch_size`) : la taille d'un lot vaut *plus grande table / nombre de threads*. Avec des tables de tailles proches, chacune est découpée en autant de lots que de threads : 16 000 tâches minuscules par système et par tick. Analogie : un répartiteur de charge qui ouvrirait une connexion par requête de quelques octets.

**3. Une taille de lot minimale règle le problème.** Avec un lot d'au moins 4 096 entités, Bevy regroupe les petites tables dans un même lot : 1 024 archétypes ne coûtent plus que +0,035 ms par tick sur une tile parisienne, et +20 % à 1 million. Effet nul quand il n'y a qu'une grosse table. Le seuil exact compte peu : à partir de 1 024, le résultat est le même.

**4. Mémoire : croissance avec le nombre d'archétypes, probablement due à la méthode** (à vérifier). Les entités sont créées dans une table, puis déplacées marqueur par marqueur ; les tables vidées gardent leur capacité. À 1 million d'entités, la mémoire passe de 127 Mo (1 archétype) à 746 Mo (1 024). Dans le jeu, une entité serait créée directement avec tous ses composants.

### Conclusion du test 2

- Les types d'entités variés sont **acceptables**, à condition de **toujours fixer une taille de lot minimale** sur les requêtes parallèles. Sans ce réglage, la fragmentation annule le gain du multi-cœur, surtout sur les petites tiles.
- Le coût du **changement d'archétype** (ajouter ou retirer un composant) est mesuré au test 3.

## Test 3 — États fréquents : marqueur ou champ

### Question

Un état comme « blessé », « en feu » ou « en fuite » peut être porté de deux façons :

- **Marqueur** : un composant sans données (`EnFeu`) ajouté ou retiré. L'entité change de table (d'archétype), ce qui copie toute sa ligne. Ces changements passent par des commandes différées, appliquées en série à la fin du tick. En échange, un système peut ne parcourir que les entités en feu.
- **Champ** : un composant `Etat { en_feu }` sur toutes les entités. Changer d'état est une simple écriture ; mais le système de traitement parcourt toutes les entités pour trouver celles qui sont en feu.

Analogie : le marqueur, c'est déplacer une ligne d'une table SQL à une autre ; le champ, c'est un `UPDATE` d'une colonne booléenne.

### Méthode

Entités avec un identifiant, une position, une vitesse et des jauges groupées ; 5 % en feu en permanence (autant d'allumages que d'extinctions). Deux systèmes enchaînés : brûler (fatigue et moral des entités en feu) et basculer (allumage ou extinction, décidés par un hachage déterministe). Bevy multi-thread, 16 threads, lot ≥ 4 096. On fait varier la part des entités qui changent d'état à chaque tick. Un test vérifie que les deux représentations donnent exactement le même état final.

### Mesures

Temps par tick en ms : **médiane (p99)**. Script : `./run-etats.sh` ([resultats-etats.csv](resultats-etats.csv)).

#### 20 000 entités (une tile parisienne)

| Changements par tick | Entités changées | Marqueur | Champ |
|---|---|---|---|
| 0 | 0 | 0,060 (0,079) | 0,061 (0,092) |
| 0,01 % | 2 | 0,063 (0,078) | 0,062 (0,079) |
| 0,1 % | 20 | 0,069 (0,094) | 0,061 (0,079) |
| 1 % | 200 | 0,105 (0,149) | 0,062 (0,079) |
| 10 % | 2 000 | 0,352 (0,439) | 0,066 (0,084) |

#### 1 million d'entités

| Changements par tick | Entités changées | Marqueur | Champ |
|---|---|---|---|
| 0 | 0 | 0,664 (0,945) | 0,560 (1,05) |
| 0,01 % | 100 | 0,736 (1,02) | 0,560 (1,02) |
| 0,1 % | 1 000 | 1,19 (1,57) | 0,559 (1,08) |
| 1 % | 10 000 | 5,50 (5,89) | 0,582 (1,28) |
| 10 % | 100 000 | 43,63 (48,88) | 0,723 (1,34) |

### Analyse

**1. Le champ ne coûte presque rien, quel que soit le rythme des changements** : de 0,56 à 0,72 ms à 1 million d'entités, entre 0 et 10 % de changements par tick.

**2. Chaque changement de marqueur coûte environ 0,15 à 0,5 µs**, appliqué en série : ajouter des cœurs n'y change rien. À 1 million d'entités, 1 % de changements par tick (10 000 entités) ajoute ≈ 5 ms ; 10 % ajoute ≈ 43 ms, au-delà du budget de ×1. À 0,1 % ou moins, le surcoût est faible.

**3. L'avantage attendu du marqueur, ne parcourir que les entités en feu, n'apparaît pas ici** : le traitement est léger, et lire un booléen sur toutes les entités coûte peu. Il deviendrait réel pour un traitement lourd réservé à une petite minorité d'entités, avec des changements rares.

### Conclusion du test 3

- **Champ** pour les états qui changent souvent (blessé, en feu, en fuite, à couvert…).
- **Marqueur** pour ce qui change rarement et définit ce qu'est l'entité : type (soldat, civil, zombie, véhicule), mort, transformation en zombie. Ordre de grandeur : un marqueur reste acceptable tant que moins de 0,1 % des entités en changent à chaque tick.

## Conclusion

- **Bevy ECS est viable comme socle de simulation** : en multi-cœur, il fait aussi bien que du code écrit à la main, à condition de grouper les composants mis à jour ensemble et de contourner la détection de changement sur les données qui changent à chaque tick.
- **La limite réelle est la bande passante mémoire** : ≈ 1 million d'entités simples par tick à ×10, quelques millions à ×1. Toute la conception doit viser à **réduire le volume de données touché par tick** : tiles inertes (A-01), et jauges lentes (faim, soif) mises à jour moins souvent que les positions.
- **Types d'entités variés** : acceptables si l'on fixe une taille de lot minimale sur les requêtes parallèles (test 2).
- **États** : un champ pour ce qui change souvent ; un marqueur seulement pour ce qui change rarement (test 3).
- **Références pour les autres prototypes** : ≈ 2 ms par tick pour 1 million d'entités simples (6 champs) en multi-cœur ; ≈ 0,1 ms pour 100 000.

## Décisions

- **Bevy retenu** comme moteur complet ; la simulation utilise l'exécuteur multi-thread ([F-01](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-01--moteur)).
- **Composants groupés** : les données mises à jour ensemble sont dans un même composant ([F-02](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation)).
- **Test à plusieurs types d'entités** : fait (test 2). Toute requête parallèle fixe une taille de lot minimale ([F-02](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation)).
- **États** : champ pour les états fréquents, marqueur pour les changements rares ([F-02](https://github.com/fafapiti/documentation/blob/main/OPEN-PROBLEMS.md#f-02--performances-de-la-simulation)).
- **Événements ponctuels** (mort d'un civil, chef tué) : mis en file, appliqués à la mise à jour suivante des jauges ([SYSTEMES.md](https://github.com/fafapiti/documentation/blob/main/game-elements/systemes/SYSTEMES.md#soldats)).
- **Jauges lentes** (fatigue, moral, faim, soif) : mises à jour une fois par seconde de jeu ([SYSTEMES.md](https://github.com/fafapiti/documentation/blob/main/game-elements/systemes/SYSTEMES.md#soldats)).

## Questions ouvertes

- Coût de la détection de changement quand on en a vraiment besoin (réaction à un changement d'état) : à mesurer dans un prototype qui l'utilise.
