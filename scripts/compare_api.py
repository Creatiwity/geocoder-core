"""End-to-end comparison with the BAN API (addok in production).

Runs the same queries against the Géoplateforme geocoder, which serves the
BAN API (api-adresse.data.gouv.fr redirects there), and
`geocoder-search --batch`, then reports how often the first result is the
same, the top-5 overlap and the score gap. Mismatches are listed.

    python3 scripts/compare_api.py data/index-ban.bin \\
        --data data/adresses-addok-france.ndjson.gz --sample 200
"""
import argparse
import gzip
import json
import random
import subprocess
import time
import urllib.parse
import urllib.request

API = "https://data.geopf.fr/geocodage/search/"

HANDPICKED = [
    "20 avenue des champs élysées paris",
    "42 rue de rivoli paris",
    "7bis rue du faubourg saint antoine paris",
    "rue de la paix paris",
    "av victor hugo paris",
    "bd haussmann paris",
    "bordeaux",
    "69001 lyon",
    "rue de la rpublique lyon",
    "impasse du chateua versailles",
    "rue nationale",
    "champs elysees paris",
    "8 bd du port",
    "mairie de lille",
    "1 place de la comedie montpellier",
    "10 rue de la gare 67000 strasbourg",
    "Société XYZ bâtiment B 22 rue des Fleurs 59350 Lille Cedex 23",
    "marseille",
    "saint etienne",
    "st etienne",
    "3 r du general de gaulle",
    "toulouse capitole",
    "rue de paris",
    "nantes",
    "gare de lyon",
    "quai de la tournelle",
    "12 bis av jean jaures 69007 lyon",
    "rennes 35000",
    "chemin des vignes",
    "grande rue",
]

ABBREVIATIONS = {"avenue": "av", "boulevard": "bd", "saint": "st", "place": "pl",
                 "impasse": "imp", "faubourg": "fg", "general": "gal", "route": "rte"}


def first(v):
    return v[0] if isinstance(v, list) and v else v


def sampled_queries(path, sample, seed):
    rnd = random.Random(seed)
    docs = []
    with gzip.open(path, "rt") as f:
        for n, line in enumerate(f):
            if n % 97 == 0:
                docs.append(line)
    rnd.shuffle(docs)
    queries = []
    for line in docs[:sample]:
        d = json.loads(line)
        name, city, postcode = first(d.get("name")), first(d.get("city")) or "", first(d.get("postcode")) or ""
        hns = list((d.get("housenumbers") or {}).keys())
        if d.get("type") == "municipality":
            queries.append(rnd.choice([name, f"{name} {postcode}", f"{postcode} {name}"]))
            continue
        hn = rnd.choice(hns) if hns else ""
        words = name.lower().split()
        variants = [
            f"{hn} {name} {postcode} {city}",
            f"{hn} {name} {city}",
            f"{name} {city}",
            " ".join(ABBREVIATIONS.get(w, w) for w in words) + f" {city}",
        ]
        q = rnd.choice(variants)
        if len(q) > 12 and rnd.random() < 0.2:
            i = rnd.randrange(3, len(q) - 3)
            q = q[:i] + q[i + 1:]  # typo
        if rnd.random() < 0.15:
            q = q[: max(8, int(len(q) * 0.7))]  # typed partially
        queries.append(" ".join(q.split()))
    return queries


def api(query, limit):
    url = API + "?" + urllib.parse.urlencode({"q": query, "limit": limit, "index": "address"})
    for attempt in range(5):
        try:
            with urllib.request.urlopen(url, timeout=20) as r:
                data = json.load(r)
            return [{"label": f["properties"]["label"], "score": f["properties"]["score"],
                     "id": f["properties"].get("id"), "type": f["properties"].get("type")}
                    for f in data["features"]]
        except Exception:  # rate limited or transient
            time.sleep(1 + attempt)
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("index")
    ap.add_argument("--data")
    ap.add_argument("--sample", type=int, default=150)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--limit", type=int, default=5)
    ap.add_argument("--bin", default="target/release/geocoder-search")
    ap.add_argument("--show", type=int, default=40)
    args = ap.parse_args()

    queries = list(HANDPICKED)
    if args.data:
        queries += sampled_queries(args.data, args.sample, args.seed)

    proc = subprocess.run([args.bin, "--batch", "--limit", str(args.limit), args.index],
                          input="\n".join(queries) + "\n", capture_output=True, text=True, check=True)
    ours = [json.loads(l)["features"] for l in proc.stdout.splitlines()]

    same_top, same_top_label, overlap, gaps, compared = 0, 0, 0.0, [], 0
    mismatches = []
    for q, mine in zip(queries, ours):
        ref = api(q, args.limit)
        time.sleep(0.05)
        if ref is None:
            continue
        compared += 1
        ref_ids = [r["id"] for r in ref]
        my_ids = [m["id"] for m in mine]
        if ref and mine and ref_ids[0] == my_ids[0]:
            same_top += 1
            gaps.append(abs(ref[0]["score"] - mine[0]["score"]))
        elif ref and mine and ref[0]["label"] == mine[0]["label"]:
            same_top_label += 1
        elif ref or mine:
            mismatches.append((q, ref, mine))
        if ref_ids or my_ids:
            overlap += len(set(ref_ids) & set(my_ids)) / max(len(set(ref_ids) | set(my_ids)), 1)
        else:
            overlap += 1
            same_top += 1

    print(f"{compared} queries compared")
    print(f"same first result (id)    : {same_top} ({100 * same_top / compared:.1f}%)")
    print(f"same first label, other id: {same_top_label}")
    print(f"top-{args.limit} overlap (Jaccard) : {overlap / compared:.3f}")
    if gaps:
        print(f"score gap on same top     : mean {sum(gaps) / len(gaps):.4f}, max {max(gaps):.4f}")
    for q, ref, mine in mismatches[: args.show]:
        print(f"\n  {q!r}")
        for r in ref[:3]:
            print(f"    BAN  [{r['score']:.3f}] {r['label']}")
        for m in mine[:3]:
            print(f"    ours [{m['score']:.3f}] {m['label']}")


if __name__ == "__main__":
    main()
