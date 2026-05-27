# Addok — Système de scoring : référence d'implémentation

Ce document décrit exhaustivement le système de scoring d'addok, avec toutes les formules, paramètres et valeurs par défaut, dans le but de permettre une réimplémentation exacte dans un autre langage.

---

## Table des matières

1. [Vue d'ensemble](#vue-densemble)
2. [Pipeline de scoring](#pipeline-de-scoring)
3. [Pré-traitement du texte (ASCII folding)](#pré-traitement-du-texte-ascii-folding)
4. [Fonctions de comparaison textuelle](#fonctions-de-comparaison-textuelle)
5. [Fonctions de scoring](#fonctions-de-scoring)
6. [Agrégation du score final](#agrégation-du-score-final)
7. [Paramètres de configuration (valeurs par défaut)](#paramètres-de-configuration-valeurs-par-défaut)
8. [Boost à l'indexation](#boost-à-lindexation)
9. [Pipeline de géocodage inverse](#pipeline-de-géocodage-inverse)
10. [Exemple de calcul complet](#exemple-de-calcul-complet)

---

## Vue d'ensemble

Chaque résultat reçoit un score final entre 0 et 1, calculé comme une **moyenne pondérée** de composantes partielles. Chaque composante a une valeur et un plafond (*ceiling*). Le score final est :

```
score_final = Σ(valeurs) / Σ(plafonds)
```

Les composantes possibles sont : `importance`, `str_distance`, `geo_distance`.

---

## Pipeline de scoring

L'ordre exact d'application est défini par `SEARCH_RESULT_PROCESSORS_PYPATHS` dans `addok/config/default.py` :

| Étape | Fonction | Fichier source |
|-------|----------|----------------|
| 1 | `match_housenumber` | `addok/helpers/results.py:32` |
| 2 | `make_labels` | `addok/helpers/results.py:13` |
| 3 | `score_by_importance` | `addok/helpers/results.py:51` |
| 4 | `score_by_autocomplete_distance` | `addok/helpers/results.py:60` |
| 5 | `score_by_ngram_distance` | `addok/helpers/results.py:101` |
| 6 | `score_by_geo_distance` | `addok/helpers/results.py:107` |
| 7 | `adjust_scores` | `addok/helpers/results.py:121` |

Si une étape retourne `False`, le résultat est **éliminé** (non inclus dans la réponse).

> **Note :** `score_by_str_distance` existe dans le code mais n'est **pas** dans le pipeline par défaut.

---

## Pré-traitement du texte (ASCII folding)

Avant toute comparaison textuelle, les chaînes sont normalisées via la classe `ascii` (`addok/helpers/text.py:136`).

**Étapes dans l'ordre :**

1. Conversion en minuscules : `value.lower()`
2. Translittération Unicode → ASCII via `unidecode` (ex : `"é"` → `"e"`, `"ñ"` → `"n"`)
3. Remplacement de tous les caractères non alphanumériques par des espaces : `re.sub(r"[^\w]", " ", text)`
4. Suppression des espaces multiples : `re.sub(r" {2,}", " ", text).strip()`

**Exemples :**
```
"Rue de l'Église"  →  "rue de l eglise"
"Saint-Étienne"    →  "saint etienne"
"Île-de-France"    →  "ile de france"
```

> Le résultat est mis en cache sur l'objet pour éviter les recalculs.

---

## Fonctions de comparaison textuelle

### `ngrams(text, n=3)` — `addok/helpers/text.py:158`

Génère l'ensemble des n-grammes d'une chaîne, avec padding d'un espace de chaque côté.

```
text_padded = " " + text + " "
ngrams = { text_padded[i:i+n]  pour i dans range(0, len(text_padded) - (n-1)) }
```

**Paramètre :** `n=3` (trigrammes de caractères).  
**Résultat :** un `set` de chaînes de longueur 3.

**Exemple :**
```
ngrams("rue") = {" ru", "rue", "ue "}
ngrams("paris") = {" pa", "par", "ari", "ris", "is "}
```

---

### `compare_str(label, query)` — `addok/helpers/text.py:175`

Comparaison hybride basée sur les 3-grammes et la distance d'édition.  
Les deux chaînes sont d'abord ASCII-foldées.

**Formule :**

```
left  = ascii(label)
right = ascii(query)

left_n  = ngrams(left)   # set de 3-grammes du label
right_n = ngrams(right)  # set de 3-grammes de la requête

inter = |left_n ∩ right_n|

# Distance d'édition normalisée sur la portion commune
distance = (editdistance(left, right) - |len(left) - len(right)|) / max(len(left), len(right))

score = inter / |right_n| * 0.85
      + inter / |left_n|  * 0.05
      + (1 - distance)    * 0.10
```

**Décomposition des poids :**

| Composante | Poids | Description |
|-----------|-------|-------------|
| Rappel n-grammes | 0.85 | Fraction des 3-grammes de la **requête** présents dans le label |
| Précision n-grammes | 0.05 | Fraction des 3-grammes du **label** présents dans la requête |
| Distance d'édition | 0.10 | `1 - distance_normalisée` |

> `editdistance` : distance de Levenshtein (bibliothèque `editdistance`).

---

### `compare_ngrams(label, query, N=2, pad_len=0)` — `addok/helpers/text.py:165`

Comparaison par bigrammes via la bibliothèque `ngram` (Dice coefficient).  
Les deux chaînes sont d'abord ASCII-foldées.

**Cas spécial :** si les deux chaînes font exactement 1 caractère, retourne `1.0` si égales, `0.0` sinon (NGram.compare retournerait toujours 0 pour des chaînes à 1 caractère).

**Formule générale (Dice sur les bigrammes) :**

```
left  = ascii(label)
right = ascii(query)

bigrammes_left  = tous les bigrammes de left
bigrammes_right = tous les bigrammes de right

score = 2 * |bigrammes_left ∩ bigrammes_right| / (|bigrammes_left| + |bigrammes_right|)
```

> C'est le coefficient de Dice/Sørensen appliqué aux bigrammes.  
> `NGram.compare` avec `pad_len=0` : aucun padding ajouté.  
> C'est cette fonction (bigrammes) qui est utilisée dans le pipeline par défaut.

---

## Fonctions de scoring

### Étape 1 — `match_housenumber` (`results.py:32`)

Tente de faire correspondre les tokens numéro-de-rue de la requête avec les données du résultat.

- Les tokens housenumber sont concaténés dans leur ordre de position.
- Si une correspondance est trouvée dans `result.housenumbers`, le résultat est mis à jour.
- Si `helper.only_housenumber=True` et qu'aucune correspondance n'est trouvée, retourne `False` → résultat éliminé.

**N'affecte pas directement les scores**, mais peut éliminer un résultat ou modifier ses coordonnées/type.

---

### Étape 2 — `make_labels` (`results.py:13`)

Construit la liste `result.labels` utilisée pour toutes les comparaisons textuelles.

**Algorithme :**

```
label = result[NAME_FIELD][0]  # ex : "Rue de la Paix"

if city exists AND city != label:
    if postcode exists:
        label = label + " " + postcode
    label = label + " " + city
    # ex : "Rue de la Paix 75001 Paris"

if housenumber exists:
    label = housenumber + " " + label
    # ex : "12 Rue de la Paix 75001 Paris"

result.labels[0] = label   # remplace le premier label
# les autres valeurs brutes du champ NAME_FIELD restent dans labels[1:]
```

---

### Étape 3 — `score_by_importance` (`results.py:51`)

```
importance_value = float(result.importance) or 0.0

score    = importance_value * IMPORTANCE_WEIGHT
ceiling  = IMPORTANCE_WEIGHT

result._scores["importance"] = (score, ceiling)
```

**Valeurs par défaut :** `IMPORTANCE_WEIGHT = 0.1`  
**Plage de score :** `[0, 0.1]`

> `result.importance` vient du document indexé. Si absent ou nul, vaut `0.0`.

---

### Étape 4 — `score_by_autocomplete_distance` (`results.py:60`)

**Uniquement activée si `helper.autocomplete = True`.**  
Si autocomplete est `False`, la fonction retourne immédiatement sans rien faire.

Pour chaque label (après ASCII folding de la **requête** et du **label**) :

| Condition | Score |
|-----------|-------|
| `label == query` (égalité exacte) | `1.0` |
| `label.startswith(query)` (le label commence par la requête) | `0.9` |
| `query in label` (la requête est contenue dans le label) | `0.7` |

- `ceiling = 1.0`
- Les conditions sont évaluées **dans cet ordre**. La première vraie s'applique.
- Seul le **meilleur score** est conservé (via `add_score`, cf. ci-dessous).
- Si **aucun des labels** ne satisfait une condition, on repasse sur `_score_by_ngram_distance` avec `scale=0.9` (fallback bigrammes ×0.9).

---

### Étape 5 — `score_by_ngram_distance` (`results.py:101`)

**Uniquement activée si `helper.autocomplete = False`.**  
Si autocomplete est `True`, la fonction retourne immédiatement.

Appelle `_score_by_ngram_distance(helper, result, scale=1.0)` :

```python
for label in result.labels:
    label = ascii(label)
    score = compare_ngrams(label, helper.query) * scale
    result.add_score("str_distance", score, ceiling=1.0)
    if score >= MATCH_THRESHOLD:
        break   # arrêt anticipé si score suffisamment bon
```

- `scale = 1.0` en mode normal
- **Arrêt anticipé** si le score dépasse `MATCH_THRESHOLD = 0.9`

---

### Étape 6 — `score_by_geo_distance` (`results.py:107`)

**Uniquement activée si `helper.lat` et `helper.lon` sont fournis.**

```python
km = haversine_distance((result.lat, result.lon), (helper.lat, helper.lon))
result.distance = km * 1000   # stocké en mètres pour l'affichage

score   = km_to_score(km) * GEO_DISTANCE_WEIGHT
ceiling = GEO_DISTANCE_WEIGHT

result._scores["geo_distance"] = (score, ceiling)
```

**`haversine_distance((lat1, lon1), (lat2, lon2))` — `helpers/__init__.py:73`**

```
lon1, lat1, lon2, lat2 = en radians

dlon = lon2 - lon1
dlat = lat2 - lat1

a = sin(dlat/2)² + cos(lat1) × cos(lat2) × sin(dlon/2)²
c = 2 × arcsin(√a)

km = 6367 × c
```

> Rayon terrestre utilisé : **6367 km**.

**`km_to_score(km)` — `helpers/__init__.py:95`**

```
si km > 100 : retourner 0.0
sinon       : retourner exp(-( km / 50.0 )²)
```

Courbe gaussienne centrée en 0 km, décroissante :

| Distance | `km_to_score` | Score final (× 0.1) |
|----------|--------------|---------------------|
| 0 km | 1.000 | 0.100 |
| 10 km | 0.960 | 0.096 |
| 25 km | 0.779 | 0.078 |
| 50 km | 0.368 | 0.037 |
| 75 km | 0.105 | 0.011 |
| 100 km | 0.018 | 0.002 |
| > 100 km | 0.000 | 0.000 |

**Valeur par défaut :** `GEO_DISTANCE_WEIGHT = 0.1`  
**Plage de score :** `[0, 0.1]`

---

### Étape 7 — `adjust_scores` (`results.py:121`)

**Uniquement activée si `helper.lat` et `helper.lon` sont fournis** (même condition que l'étape 6).

Quand des coordonnées sont fournies, la distance géographique devient plus importante, donc les autres composantes sont réévaluées à la baisse :

```python
if "str_distance" in result._scores:
    (score, ceiling) = result._scores["str_distance"]
    result._scores["str_distance"] = (score * 0.9, ceiling)   # × 0.9

if "importance" in result._scores:
    (score, ceiling) = result._scores["importance"]
    result._scores["importance"] = (score * 0.1, ceiling)     # × 0.1
```

> Attention : seule la **valeur** est modifiée, pas le **plafond** (*ceiling*). Le plafond reste inchangé, ce qui dilue le poids effectif de ces composantes dans l'agrégation finale.

---

## Agrégation du score final

### `add_score(name, score, ceiling)` — `core.py:92`

À chaque fois qu'une composante est mise à jour, seul le **score le plus élevé** est conservé :

```python
if score >= current_score_for_name:
    _scores[name] = (score, ceiling)
```

Cela signifie que si plusieurs labels sont comparés, seul le meilleur score par composante est retenu.

### `result.score` — `core.py:96`

```python
score_final = Σ(scores) / Σ(ceilings)
```

Où `_scores` est un dictionnaire `{name: (valeur, plafond)}`.

**Exemple avec toutes les composantes actives et coordonnées fournies :**

```
_scores = {
  "importance":   (0.050 × 0.1, 0.1)  →  (0.005, 0.1)   → après adjust: (0.0005, 0.1)
  "str_distance": (0.82,        1.0)               → après adjust: (0.738,  1.0)
  "geo_distance": (0.960 × 0.1, 0.1)  →  (0.096, 0.1)
}

score_final = (0.0005 + 0.738 + 0.096) / (0.1 + 1.0 + 0.1)
            = 0.8345 / 1.2
            = 0.695
```

### Seuil de score minimum

Les résultats avec `score_final < MIN_SCORE` sont exclus de la réponse.  
**Valeur par défaut :** `MIN_SCORE = 0.1`

### Score forcé

Si `result.score` est défini manuellement via le setter (ex : par un plugin), la valeur explicite est retournée directement, sans calcul. Par défaut, `_score` est `""` (chaîne vide), ce qui déclenche le calcul automatique.

---

## Paramètres de configuration (valeurs par défaut)

Source : `addok/config/default.py`

### Poids de scoring

| Paramètre | Valeur | Rôle |
|-----------|--------|------|
| `IMPORTANCE_WEIGHT` | `0.1` | Plafond et poids du score d'importance |
| `GEO_DISTANCE_WEIGHT` | `0.1` | Plafond et poids du score géographique |
| `DEFAULT_BOOST` | `1.0` | Score de boost par défaut token→document à l'indexation |

### Seuils

| Paramètre | Valeur | Rôle |
|-----------|--------|------|
| `MATCH_THRESHOLD` | `0.9` | Seuil d'arrêt anticipé dans `_score_by_ngram_distance` |
| `MIN_SCORE` | `0.1` | Score minimum pour inclure un résultat |
| `COMMON_THRESHOLD` | `10000` | Fréquence au-delà de laquelle un token est "commun" |
| `INTERSECT_LIMIT` | `100000` | Au-delà, on évite les intersections de sets Redis |

### Récupération des résultats

| Paramètre | Valeur | Rôle |
|-----------|--------|------|
| `BUCKET_MIN` | `10` | Nombre minimum de documents à récupérer et scorer |
| `BUCKET_MAX` | `100` | Nombre maximum de documents à récupérer et scorer |

### Traitement textuel

| Paramètre | Valeur | Rôle |
|-----------|--------|------|
| `MIN_EDGE_NGRAMS` | `3` | Longueur minimale des edge n-grammes (autocomplete) |
| `MAX_EDGE_NGRAMS` | `20` | Longueur maximale des edge n-grammes |
| `QUERY_MAX_LENGTH` | `200` | Longueur maximale de la requête (en caractères) |

### Champs indexés (avec boost)

| Champ | Boost | Obligatoire |
|-------|-------|-------------|
| `name` | `4` | Oui (`null: False`) |
| `street` | `1.0` (défaut) | Non |
| `postcode` | `1.2` si type=`municipality`, sinon `1.0` | Non |
| `city` | `1.0` (défaut) | Non |
| `housenumbers` | — (indexé séparément) | Non |
| `context` | `1.0` (défaut) | Non |

---

## Boost à l'indexation

Source : `addok/helpers/index.py:30` (`extract_tokens`) et `addok/helpers/index.py:123` (`FieldsIndexer.index`)

Le boost d'un token dans le sorted set Redis est calculé ainsi :

```python
importance = float(doc.get("importance", 0.0)) * IMPORTANCE_WEIGHT

for field in FIELDS:
    boost = field.get("boost", DEFAULT_BOOST)    # ex: 4 pour "name"
    if callable(boost):
        boost = boost(doc)                        # ex: lambda pour postcode
    boost = boost + importance                    # ajout de l'importance du doc

    els = preprocess(field_value)                 # tokenisation + normalisation
    token_boost = DEFAULT_BOOST / len(els) * boost

    # Pour chaque token, on ne garde que le boost max déjà vu
    if token_boost > tokens.get(token, 0):
        tokens[token] = token_boost
```

Le boost est ensuite stocké comme score dans le sorted set Redis : `ZADD token_key {doc_key: boost}`.

> Ce boost conditionne l'**ordre de récupération** des candidats (quels documents sont renvoyés pour un token donné), mais n'affecte pas directement le score de pertinence calculé à l'étape de scoring.

---

## Pipeline de géocodage inverse

Source : `addok/config/default.py:102` (`REVERSE_RESULT_PROCESSORS_PYPATHS`)

Pour le géocodage inverse (recherche par coordonnées), seules trois étapes sont appliquées :

| Étape | Fonction | Description |
|-------|----------|-------------|
| 1 | `load_closer` | Trouve le numéro de rue le plus proche des coordonnées |
| 2 | `make_labels` | Génère les labels d'affichage |
| 3 | `score_by_geo_distance` | Score uniquement par distance géographique |

Pas de scoring textuel. Le score final = `km_to_score(km) * GEO_DISTANCE_WEIGHT / GEO_DISTANCE_WEIGHT` = `km_to_score(km)`.

---

## Exemple de calcul complet

**Requête :** `"12 rue de la paix paris"` avec coordonnées `lat=48.8698, lon=2.3298`  
**Résultat candidat :** document `{name: "Rue de la Paix", city: "Paris", postcode: "75001", importance: 0.5, lat: 48.8699, lon: 2.3301}`

**Étape 1 — match_housenumber :**  
Token `"12"` trouvé dans les housenumbers → `result.housenumber = "12"`, `result.type = "housenumber"`, coordonnées mises à jour sur le numéro.

**Étape 2 — make_labels :**  
`result.labels[0] = "12 Rue de la Paix 75001 Paris"`

**Étape 3 — score_by_importance :**  
`score = 0.5 × 0.1 = 0.05`, `ceiling = 0.1`  
→ `_scores["importance"] = (0.05, 0.1)`

**Étape 4 — score_by_autocomplete_distance :**  
`helper.autocomplete = False` → retour immédiat, rien.

**Étape 5 — score_by_ngram_distance :**  
`compare_ngrams(ascii("12 Rue de la Paix 75001 Paris"), ascii("12 rue de la paix paris"))`  
= `compare_ngrams("12 rue de la paix 75001 paris", "12 rue de la paix paris")`  
≈ `0.93` (score élevé, arrêt anticipé car ≥ 0.9)  
→ `_scores["str_distance"] = (0.93, 1.0)`

**Étape 6 — score_by_geo_distance :**  
`km = haversine_distance(...)` ≈ `0.025 km`  
`km_to_score(0.025) = exp(-(0.025/50)²) ≈ 1.000`  
`score = 1.000 × 0.1 = 0.100`, `ceiling = 0.1`  
→ `_scores["geo_distance"] = (0.100, 0.1)`

**Étape 7 — adjust_scores (lat/lon présents) :**  
`_scores["str_distance"] = (0.93 × 0.9, 1.0) = (0.837, 1.0)`  
`_scores["importance"]   = (0.05 × 0.1, 0.1) = (0.005, 0.1)`

**Score final :**
```
numerateur   = 0.837 + 0.005 + 0.100 = 0.942
denominateur = 1.0   + 0.1   + 0.1   = 1.2

score_final  = 0.942 / 1.2 = 0.785
```

---

## Dépendances bibliothèques

| Bibliothèque | Usage |
|-------------|-------|
| `unidecode` | Translittération Unicode → ASCII |
| `editdistance` | Distance de Levenshtein |
| `ngram` (classe `NGram`) | Coefficient de Dice sur bigrammes |
| `geohash` | Encodage/décodage géohash (précision 7 par défaut) |
