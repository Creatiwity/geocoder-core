#!/usr/bin/env bash
# Compare geocoder-core results against the BAN API (addok reference).
# Usage: ./scripts/compare.sh <index.bin>

set -euo pipefail
INDEX=${1:?usage: compare.sh <index.bin>}
BIN="$(dirname "$0")/../target/release/geocoder-search"
API="https://api-adresse.data.gouv.fr/search"

urlencode() { python3 -c "import urllib.parse,sys; print(urllib.parse.quote(sys.argv[1]))" "$1"; }

run() {
    local label="$1" query="$2" extra_cli="${3:-}" extra_api="${4:-}"
    printf '\n\033[1m══ %s\033[0m\n   query: "%s"\n\n' "$label" "$query"

    printf '\033[33m── addok (BAN API) ──\033[0m\n'
    curl -s "${API}/?q=$(urlencode "$query")&limit=5${extra_api}" \
      | python3 -c "
import json,sys
for f in json.load(sys.stdin)['features']:
    p=f['properties']
    hn=(' hn='+p['housenumber']) if 'housenumber' in p else ''
    print(f'  [{p[\"score\"]:.3f}] {p[\"label\"]}{hn}')
"

    printf '\n\033[36m── geocoder-core ────\033[0m\n'
    "$BIN" $extra_cli "$INDEX" "$query"
}

# Adresse avec numéro
run "Adresse complète — rue célèbre"     "20 avenue des champs élysées paris"
run "Adresse — voie ordinaire"           "42 rue de rivoli paris"
run "Numéro bis"                         "7bis rue du faubourg saint antoine paris"

# Voie sans numéro
run "Voie + ville"                       "rue de la paix paris"
run "Synonyme av → avenue"              "av victor hugo paris"
run "Synonyme bd → boulevard"           "bd haussmann paris"

# Lieu / code postal
run "Ville seule"                        "bordeaux"
run "Code postal + ville"                "69001 lyon"

# Fuzzy
run "Faute légère — lettre manquante"   "rue de la rpublique lyon"
run "Faute légère — transposition"      "impasse du chateua versailles"

# Géo hint
run "Rue ambiguë sans hint"             "rue nationale" "" ""
run "Même requête, hint Lille"          "rue nationale" \
    "--lat 50.6292 --lon 3.0573" "&lat=50.6292&lon=3.0573"

# Filtre
run "Filtre type=street"                "champs elysees paris" "--filter type=street"
