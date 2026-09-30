"""Reference oracle for the differential tests: the real addok text pipeline,
index boosts and result scoring, configured like the address index of the
Géoplateforme geocoder (which serves the BAN API): addok 1.3.2 +
addok-fr 1.1.0 + addok-france 1.2.0. JSON lines on stdin/stdout, no Redis.

Setup:

    uv venv -p 3.12 .oracle
    VIRTUAL_ENV=.oracle uv pip install addok==1.3.2 addok-fr==1.1.0 addok-france==1.2.0
    .oracle/bin/python scripts/addok_oracle.py
"""
import json
import os
import sys
import tempfile

# geoplateforme/geocodage/geocodeur, indexes/address/config/addok.conf
ADDOK_CONFIG = '''
EXTRA_FIELDS = [
    {"key": "citycode"}
]
FILTERS = ["source", "type", "citycode", "postcode"]
QUERY_PROCESSORS_PYPATHS = [
    "addok.helpers.text.check_query_length",
    "addok_france.extract_address",
    "addok_france.clean_query",
    "addok_france.remove_leading_zeros",
]
SEARCH_RESULT_PROCESSORS_PYPATHS = [
    "addok.helpers.results.match_housenumber",
    "addok_france.make_labels",
    "addok.helpers.results.score_by_importance",
    "addok.helpers.results.score_by_autocomplete_distance",
    "addok.helpers.results.score_by_ngram_distance",
    "addok.helpers.results.score_by_geo_distance",
]
PROCESSORS_PYPATHS = [
    "addok.helpers.text.tokenize",
    "addok.helpers.text.normalize",
    "addok_france.glue_ordinal",
    "addok_france.fold_ordinal",
    "addok_france.flag_housenumber",
    "addok.helpers.text.synonymize",
    "addok_fr.phonemicize",
]
MIN_SCORE = 0.3
'''

_conf = tempfile.NamedTemporaryFile("w", suffix=".conf", delete=False)
_conf.write(ADDOK_CONFIG)
_conf.close()
os.environ["ADDOK_CONFIG_MODULE"] = _conf.name

# Keep stdout for the protocol: addok prints while loading its config.
_stdout = sys.stdout
sys.stdout = sys.stderr
import warnings  # noqa: E402

warnings.simplefilter("ignore")
from addok.config import config  # noqa: E402

config.load()

from addok.core import Result  # noqa: E402
from addok.helpers import iter_pipe  # noqa: E402
from addok.helpers.index import extract_tokens, prepare_housenumbers  # noqa: E402
from addok.helpers.text import ascii  # noqa: E402

sys.stdout = _stdout


def index_tokens(doc):
    # FieldsIndexer.index, without Redis.
    tokens = {}
    importance = float(doc.get("importance", 0.0)) * config.IMPORTANCE_WEIGHT
    for field in config.FIELDS:
        name = field["key"]
        values = doc.get(name)
        if not values or name == config.HOUSENUMBERS_FIELD:
            continue
        boost = field.get("boost", config.DEFAULT_BOOST)
        if callable(boost):
            boost = boost(doc)
        boost = boost + importance
        if not isinstance(values, (list, tuple)):
            values = [values]
        for value in values:
            extract_tokens(tokens, str(value), boost=boost)
    return {str(k): v for k, v in tokens.items()}


def query_tokens(q):
    tokens = list(iter_pipe(ascii(q.strip()), config.QUERY_PROCESSORS + config.PROCESSORS))
    return [{"t": str(t), "kind": t.kind, "pos": list(t.position)} for t in tokens]


class FakeHelper:
    def __init__(self, q, autocomplete, lat, lon, filters):
        self.query = ascii(q.strip())
        self.autocomplete = autocomplete
        self.lat = lat
        self.lon = lon
        self.check_housenumber = filters.get("type") in [None, "housenumber"]
        self.only_housenumber = filters.get("type") == "housenumber"
        self.tokens = list(iter_pipe(self.query, config.QUERY_PROCESSORS + config.PROCESSORS))
        self.housenumbers = [t for t in self.tokens if t.kind == "housenumber"]

    def debug(self, *args):
        pass


def score(q, doc, autocomplete=True, lat=None, lon=None, filters=None):
    doc = list(prepare_housenumbers([json.loads(json.dumps(doc))]))[0]
    helper = FakeHelper(q, autocomplete, lat, lon, filters or {})
    result = Result(doc)
    for processor in config.SEARCH_RESULT_PROCESSORS:
        if processor(helper, result) is False:
            return None
    return {
        "labels": [str(label) for label in result.labels],
        "score": result.score,
        "housenumber": result.housenumber,
        "label": str(result),
    }


def main():
    for line in sys.stdin:
        req = json.loads(line)
        op = req["op"]
        if op == "tokens":
            out = [str(t) for t in iter_pipe(req["s"], config.PROCESSORS)]
        elif op == "query":
            out = query_tokens(req["q"])
        elif op == "index":
            out = index_tokens(req["doc"])
        elif op == "hn":
            out = "".join(str(t) for t in iter_pipe(req["s"], config.PROCESSORS))
        elif op == "score":
            out = score(req["q"], req["doc"], req.get("autocomplete", True),
                        req.get("lat"), req.get("lon"), req.get("filters"))
        else:
            raise ValueError(op)
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
