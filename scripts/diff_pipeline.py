"""Differential test of the Rust pipeline against addok (scripts/addok_oracle.py).

Samples real documents from an addok-format export, derives queries from
them (full, abbreviated, with housenumbers and ordinals, typos, cedex,
leading zeros…) and compares, op by op, what both implementations return.

    python3 scripts/diff_pipeline.py --oracle .oracle/bin/python \\
        --data data/adresses-addok-france.ndjson.gz --every 400
"""
import argparse
import gzip
import json
import random
import subprocess
import sys
from collections import Counter, defaultdict

ABBREVIATIONS = {"avenue": "av", "boulevard": "bd", "saint": "st", "place": "pl",
                 "chemin": "ch", "route": "rte", "impasse": "imp", "faubourg": "fg",
                 "general": "gal", "docteur": "dr", "rue": "r", "allee": "all"}


class Proc:
    def __init__(self, cmd):
        self.p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True, bufsize=1)

    def ask(self, req):
        self.p.stdin.write(json.dumps(req) + "\n")
        self.p.stdin.flush()
        line = self.p.stdout.readline()
        if not line:
            raise RuntimeError("process died on %r" % (req,))
        return json.loads(line)


def first(v):
    return v[0] if isinstance(v, list) and v else v


def queries_for(doc, rnd):
    name = first(doc.get("name")) or ""
    city = first(doc.get("city")) or ""
    postcode = first(doc.get("postcode")) or ""
    hns = list((doc.get("housenumbers") or {}).keys())
    qs = [name, f"{name} {city}", f"{name} {postcode} {city}", name.lower()]
    cities = doc.get("city") if isinstance(doc.get("city"), list) else []
    if len(cities) > 1:
        # Former municipality of a merged one.
        qs += [f"{name} {cities[1]}", f"{name} {postcode} {cities[-1]}"]
    if hns:
        hn = rnd.choice(hns)
        qs += [f"{hn} {name} {postcode} {city}", f"{hn} {name} {city}", f"{hn}, {name}",
               f"{hn} bis {name} {city}", f"00{hn} {name}", f"{hn} {name} {postcode} {city} cedex 12"]
    words = name.lower().split()
    abbreviated = " ".join(ABBREVIATIONS.get(w, w) for w in words)
    if abbreviated != name.lower():
        qs.append(f"{abbreviated} {city}")
    if len(name) > 5:
        i = rnd.randrange(1, len(name) - 1)
        qs.append(f"{name[:i]}{name[i + 1:]} {city}")  # missing letter
        qs.append(name[: max(3, len(name) // 2)])  # prefix, autocomplete style
    qs.append(f"Société XYZ bâtiment B {name} {postcode} {city}")
    return [q for q in qs if q.strip()]


def close(a, b, tol=1e-6):
    if isinstance(a, float) or isinstance(b, float):
        return a is not None and b is not None and abs(a - b) <= tol
    return a == b


def same_score(a, b):
    if a is None or b is None:  # rejected by a processor
        return a is b
    return close(a["score"], b["score"]) and a["label"] == b["label"] and a["housenumber"] == b["housenumber"]


def compare_index(a, b):
    if set(a) != set(b):
        return False
    return all(abs(a[k] - b[k]) < 1e-9 for k in a)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--oracle", required=True, help="python of the addok venv")
    ap.add_argument("--data", required=True)
    ap.add_argument("--every", type=int, default=400, help="keep one line out of N")
    ap.add_argument("--max-docs", type=int, default=5000)
    ap.add_argument("--examples", type=int, default=5)
    ap.add_argument("--rust", default="target/release/examples/pipeline")
    args = ap.parse_args()

    oracle = Proc([args.oracle, "scripts/addok_oracle.py"])
    rust = Proc([args.rust, "fr"])
    rnd = random.Random(42)
    counts = Counter()
    failures = defaultdict(list)

    def check(op, req, cmp=lambda a, b: a == b):
        a, b = oracle.ask(req), rust.ask(req)
        counts[op] += 1
        if not cmp(a, b):
            counts[op + " KO"] += 1
            if len(failures[op]) < args.examples:
                failures[op].append((req, a, b))

    opener = gzip.open if args.data.endswith(".gz") else open
    docs = 0
    with opener(args.data, "rt") as f:
        for n, line in enumerate(f):
            if n % args.every:
                continue
            doc = json.loads(line)
            docs += 1
            check("index", {"op": "index", "doc": doc}, compare_index)
            for key in ("name", "city", "context"):
                if doc.get(key):
                    check("tokens", {"op": "tokens", "s": str(first(doc[key]))})
            for hn in list((doc.get("housenumbers") or {}).keys())[:3]:
                check("hn", {"op": "hn", "s": hn})
            small = dict(doc)
            if small.get("housenumbers"):
                keys = list(small["housenumbers"])[:20]
                small["housenumbers"] = {k: small["housenumbers"][k] for k in keys}
            for q in queries_for(doc, rnd):
                check("query", {"op": "query", "q": q},
                      lambda a, b: [(t["t"], t["kind"]) for t in a] == [(t["t"], t["kind"]) for t in b])
                req = {"op": "score", "q": q, "doc": small}
                if rnd.random() < 0.3:
                    req["autocomplete"] = False
                if rnd.random() < 0.3 and doc.get("lat"):
                    req["lat"], req["lon"] = doc["lat"] + 0.05, doc["lon"] - 0.05
                if rnd.random() < 0.1:
                    req["filters"] = {"type": "housenumber"}
                check("score", req, same_score)
            if docs >= args.max_docs:
                break

    print(f"{docs} documents")
    ok = True
    for op in ("index", "tokens", "hn", "query", "score"):
        total, ko = counts[op], counts[op + " KO"]
        ok &= ko == 0
        print(f"{op:7} {total - ko:7}/{total:<7} identical")
        for req, a, b in failures[op]:
            print("   req   :", json.dumps(req, ensure_ascii=False)[:300])
            print("   addok :", json.dumps(a, ensure_ascii=False)[:300])
            print("   rust  :", json.dumps(b, ensure_ascii=False)[:300])
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
