# Scoring addok — référence (API Adresse / Géoplateforme)

Ce document décrit le comportement que geocoder-core reproduit : celui de
l'API Adresse, servie depuis 2024 par le géocodeur de la Géoplateforme (IGN,
`data.geopf.fr/geocodage`, vers lequel `api-adresse.data.gouv.fr` redirige).

- **Configuration** : `indexes/address/config/addok.conf` du dépôt
  `geoplateforme/geocodage/geocodeur` (miroir `gouv-nc-data/gpf-geocodeur`).
- **Versions** : **addok 1.3 + addok-fr 1.1.0 + addok-france 1.2.0**. Le
  dépôt épingle encore addok 1.1.1 / addok-fr 1.0.1 / addok-france 1.1.3
  (2023), mais l'API se comporte comme les versions de fin 2025 : filtres à
  plusieurs valeurs (addok 1.3), libellés pour les anciennes communes
  fusionnées (addok-france 1.2), phonétique d'addok-fr 1.1 (vérifié par
  sondage : des requêtes que seule la 1.1 rapproche sont trouvées).

Chaque point ci-dessous a été vérifié contre le code Python lui-même
(`scripts/addok_oracle.py`, `scripts/diff_pipeline.py`).

> Attention aux sources : `BaseAdresseNationale/addok-docker` (addok 1.0.3)
> et la configuration par défaut d'addok diffèrent de la production sur
> plusieurs points (phonétique, arrêt anticipé de l'autocomplete, ordre des
> collecteurs, champs, `MIN_SCORE`…).

## 1. Traitement du texte

### Requête (`QUERY_PROCESSORS`, sur la chaîne)

La requête est d'abord repliée en ASCII (`ascii` : minuscules, `unidecode`
1.3.6, tout caractère non-mot → espace, espaces multiples fusionnés, bords
retirés), puis :

1. `check_query_length` : 200 caractères maximum ;
2. `extract_address` : si un motif « numéro [ordinal] type de voie … » est
   trouvé, on ne garde que la partie qui commence au numéro
   (« XYZ Ets bâtiment B 22 rue des Fleurs… » → « 22 rue des Fleurs… ») ;
3. `clean_query` : codes postaux isolés, BP/CS/TSA/CIDEX, cedex (on ne garde
   alors que le département du code postal), étages, `lieu-dit` en tête ;
4. `remove_leading_zeros` : « 0003 » → « 3 » (1 à 3 chiffres seulement).

### Mots (`PROCESSORS`, à l'indexation comme à la requête)

1. `tokenize` : suites de caractères de mot au sens Python (`str.isalnum()`
   ou `_`) ;
2. `normalize` : `unidecode(minuscules)` ;
3. `glue_ordinal` : « 6 bis » → « 6bis » si suivi d'un type de voie ou de
   rien (jamais un nombre de 5 chiffres, qui est un code postal) ;
4. `fold_ordinal` : « 6bis » → « 6b » (bis→b, ter→t, quater→q,
   quinquies→c, sexies→s) ;
5. `flag_housenumber` : le premier nombre (`\d{1,4}[a-z]?`) en tête ou juste
   avant un type de voie devient le numéro ;
6. `synonymize` : remplacement (jamais ajout) selon `synonyms.txt`
   d'addok-fr, éventuellement en plusieurs mots (« cr » → « chemin rural »).
   Les nombres sont aussi concernés (« 1 » → « un », « 10 » → « dix ») ;
7. `phonemicize` : règles d'addok-fr **1.1.0** (« rue » → « ru »,
   « boulevard » → « boulevar », « champs » → « chan », « vingt » → « vin »,
   « georges » → « jorj », « lyon » → « lion »).

Les types de voie reconnus par `extract_address` et `flag_housenumber` sont
ceux d'addok-france 1.2.0, abréviations comprises (« ch », « chp », « rdpt »…).

Les mots de numéro ne sont pas cherchés dans l'index : ils servent à
sélectionner le numéro dans le document (clé = mots du numéro concaténés).

## 2. Indexation

Champs indexés et poids (`FIELDS` + `EXTRA_FIELDS`) :

| Champ | Poids |
|---|---|
| `name` (obligatoire) | 4 |
| `postcode` | 1,2 pour une commune, 1 sinon |
| `street`, `city`, `context`, `citycode` | 1 |

(`oldcity`, `oldcitycode` et `district`, indexés par addok-docker, ne le sont
pas à la Géoplateforme.)

Pour chaque valeur d'un champ découpée en *n* mots, chaque mot reçoit
`(poids + importance × 0,1) / n`, et un mot garde le meilleur poids vu dans
le document. Ce poids ne sert qu'à **choisir et ordonner les candidats**
(intersection des mots, scores sommés, 100 premiers), pas au score final.

S'ajoutent : paires de mots co-présents (fuzzy, autocomplete, relations),
préfixes d'autocomplete de 3 à 20 caractères de **tous** les mots, numériques
compris (l'import passe par `addok batch` sur l'entrée standard, qui les
calcule ; « 268 » complète donc le code postal « 26800 »), géohash à 7
caractères du document et de ses numéros, filtres `source`, `type`,
`citycode`, `postcode` (plus `type=housenumber` pour les voies numérotées).

## 3. Choix des candidats

Les mots sont classés en *fréquents* (> 10 000 documents), *significatifs*
(trouvés), *introuvables* ou *numéro*. Puis les 13 collecteurs d'addok 1.1
s'enchaînent (voir `src/search/mod.rs`), dans cet ordre : intersection des
mots significatifs (plus un mot fréquent s'il n'y en a qu'un), réduction par
les mots fréquents tant qu'il y a plus de 100 candidats, autocomplete sur le
dernier mot (candidats triés par meilleur document pour une requête d'un mot,
par fréquence sinon), voisins fuzzy à une faute (substitutions limitées aux
touches voisines d'un clavier AZERTY, validés par les paires), groupes de
mots liés (les moins fréquents d'abord), puis retrait d'un, voire de deux
mots. Les filtres acceptent jusqu'à 10 valeurs, combinées en OU. Un collecteur peut arrêter
la chaîne, notamment dès qu'un candidat a un score textuel ≥ 0,9 parmi au
plus 10.

## 4. Score d'un résultat

`SEARCH_RESULT_PROCESSORS` :

1. `match_housenumber` : si le numéro existe, le résultat prend ses
   coordonnées et son id, et son type devient `housenumber` ; avec le filtre
   `type=housenumber`, un résultat sans numéro trouvé est écarté ;
2. `make_labels` (addok-france 1.2.0) : toutes les combinaisons
   « [numéro] nom [code postal] [commune] » (et « code postal nom » pour une
   commune), pour **chaque** commune de la liste : une adresse d'une commune
   nouvelle se trouve aussi sous le nom de l'ancienne commune ;
3. `score_by_importance` : `importance × 0,1`, plafond 0,1 ;
4. `score_by_autocomplete_distance` (autocomplete actif, **défaut de l'API**) :
   sur les libellés repliés en ASCII, 1,0 si égal à la requête, 0,9 s'il
   commence par elle, 0,7 s'il la contient, **sur tous les libellés** (addok
   1.0 s'arrêtait au premier ≥ 0,9) ; sinon repli sur les n-grammes × 0,9 ;
5. `score_by_ngram_distance` (autocomplete inactif) : n-grammes × 1,0 ;
6. `score_by_geo_distance` (si centre donné) : `0,1 × exp(-(km/50)²)`, 0
   au-delà de 100 km, plafond 0,1. Distance haversine, rayon 6 367 km.

Chaque composante garde la meilleure valeur sur l'ensemble des libellés.
Score final = Σ valeurs / Σ plafonds. Pas d'`adjust_scores` à la BAN.
Les résultats sont triés, tronqués, puis ceux sous **0,3** (`MIN_SCORE` de
la Géoplateforme) sont retirés.

La Géoplateforme demande toujours **au moins 10 résultats** à addok
(`api/indexes/address.js` : `Math.max(params.limit || 10, 10)`), puis ne
renvoie que les `limit` meilleurs. Ce n'est pas qu'une question
d'affichage : les collecteurs raisonnent sur ce nombre (bucket « plein » ou
« à sec »), donc une requête avec `limit=5` peut déclencher un fuzzy qu'addok
seul, avec 5, n'aurait pas tenté. geocoder-core fait de même
(`Profile::min_limit`).

### Similarité n-grammes

`NGram.compare(libellé, requête, N=2, pad_len=0)` (bibliothèque `ngram`
4.0.3) : bigrammes sans bourrage,

```
commun = bigrammes partagés, comptés avec multiplicité
score  = commun / (bigrammes(libellé) + bigrammes(requête) − commun)
```

C'est un Jaccard sur multiensembles, pas un Dice. Deux chaînes d'un seul
caractère valent 1 si égales ; sans bigramme commun, 0.

### Exemple

« 42 rue de rivoli paris » contre la rue de Rivoli (75004, importance 0,7),
numéro 42 présent : le libellé « 42 Rue de Rivoli 75004 Paris » ne matche
pas, mais « 42 Rue de Rivoli Paris » est égal à la requête → 1,0.
Score = (1,0 + 0,07) / (1,0 + 0,1) = 0,9727.
