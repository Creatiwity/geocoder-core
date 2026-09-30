//! Port of `addok_france.utils` (addok-france 1.2.0).

use once_cell::sync::Lazy;
use regex::Regex;

use crate::doc::{Document, first_str, raw_values, str_values, truthy};
use crate::text::{Token, TokenKind, is_digit};

const TYPES: &[&str] = &[
    "aer(odrome)?",
    "all([ée]es?)?",
    "anc(ien(ne)?)?",
    "av(enue)?",
    "b(oulevar|l?v?)?d",
    "b(ou)?cle",
    "ber(ges?)?",
    "bois",
    "c(en)?tre",
    "c(ou)?rs?",
    "c[ôo]te",
    "carr?(efour)?",
    "ch(am)?p",
    "chauss[ée]e",
    "che?(m(in)?)?",
    "cit[ée]",
    "clos",
    "desserte",
    "devi(ation)?",
    "dig(ue)?",
    "dom(aine)?",
    "embr(anchement)?",
    "espl?(anade)?",
    "eta?ng",
    "éta?ng",
    "f(aubour|b|bour)?g",
    "giratoire",
    "gr(ande?)?",
    "gr(e|es|s)?",
    "ham(eau)?",
    "imp(asse)?",
    "j[ée]?t[ée]e",
    "jard(in)?",
    "l[ôo]t(issement)?",
    "mont",
    "mont[ée]e",
    "p(arvis|rvs?|vr)?",
    "p(asserel|l)?le",
    "p(lace)?tte",
    "p(or)?te",
    "parc",
    "pas(se)?",
    "pass(age)?",
    "pl(ace)?",
    "porte",
    "pr[ée]",
    "pro?m(enade)?",
    "q(uart(ier)?|ua|rt(ier)?)",
    "qu?(ai)?",
    "r(ou)?te",
    "r(uel|l)?le",
    "r(ue)?",
    "r[ée]s(idence)?",
    "rd?pt",
    "rocade",
    "rond[- ]point",
    "sent(e|ier)",
    "squ?(are)?",
    "t(erra)?sse",
    "taillis",
    "trav(erse)?",
    "tunn?(el)?",
    "v(il|l)?la",
    "via",
    "viad(uc)?",
    "voie",
    "[îi]lot",
];
const ORDINAL: &str = "bis|ter|quater|quinquies|sexies|[a-z]";

fn types() -> String {
    TYPES.join("|")
}

/// "22 rue des Fleurs 59350 Lille" out of "XYZ Ets bâtiment B 22 rue des
/// Fleurs 59350 Lille Cedex 23".
static EXTRACT_ADDRESS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)(\b\d{{1,4}}( *({ORDINAL}))?,? +({}) .*(\d{{5}})?).*",
        types()
    ))
    .unwrap()
});
/// `re.match` anchors at the start; the patterns only ever see one token.
static ORDINAL_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(&format!(r"(?i)^\b({ORDINAL})\b")).unwrap());
static TYPES_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(&format!(r"(?i)^\b({})\b", types())).unwrap());
static FOLD_PATTERN: Lazy<Regex> =
    Lazy::new(|| Regex::new(&format!(r"(?i)^(\d{{1,4}})({ORDINAL})$")).unwrap());
static NUMBER_PATTERN: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\b\d{1,4}[a-z]?\b").unwrap());
static LEADING_ZEROS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b0+(\d{1,3})\b").unwrap());
static LIEU_DIT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^lieux?[ -]?dits?\b").unwrap());

static CLEAN: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    [
        (r"([\d]{5})", " ${1} "),
        (r"(^| )(b\.?p\.?|cs|tsa|cidex) *(n(o|°|) *|)[\d]+ *", "${1}"),
        (r"([\d]{2})[\d]{3}(.*)c(e|é)dex ?[\d]*", "${1}${2}"),
        (r"c(e|é)dex ?[\d]*", ""),
        // Python's `\d{,2}` is `\d{0,2}`.
        (r"\d{0,2}(e|[eè]me) ([eé]tage)", ""),
        (" {2,}", " "),
        ("[ -]s/[ -]", " sur "),
        ("[ -]s/s[ -]", " sous "),
    ]
    .into_iter()
    .map(|(p, r)| (Regex::new(&format!("(?i){p}")).unwrap(), r))
    .collect()
});

const FOLD: &[(&str, &str)] = &[
    ("bis", "b"),
    ("ter", "t"),
    ("quater", "q"),
    ("quinquies", "c"),
    ("sexies", "s"),
];

pub fn clean_query(q: &str) -> String {
    let mut q = q.to_string();
    for (pattern, repl) in CLEAN.iter() {
        q = pattern.replace_all(&q, *repl).into_owned();
    }
    // `^lieux?[ -]?dits?\b(?=.)`: only when something follows.
    if let Some(m) = LIEU_DIT.find(&q)
        && m.end() < q.len()
    {
        q = q[m.end()..].to_string();
    }
    q.trim().to_string()
}

pub fn extract_address(q: &str) -> String {
    match EXTRACT_ADDRESS.find(q) {
        Some(m) => m.as_str().to_string(),
        None => q.to_string(),
    }
}

/// "0003" → "3", leaving postcodes alone.
pub fn remove_leading_zeros(q: &str) -> String {
    LEADING_ZEROS.replace_all(q, "${1}").into_owned()
}

/// "6 bis" → "6bis" when followed by a street type or nothing.
pub fn glue_ordinal(tokens: Vec<Token>) -> Vec<Token> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut previous: Option<Token> = None;
    for (i, token) in tokens.iter().enumerate() {
        let next = tokens.get(i + 1);
        // Not a postcode (5 digits). A number followed by another number
        // drops the first one, like addok-france does.
        if next.is_some() && is_digit(token) && token.chars().count() < 5 {
            previous = Some(token.clone());
            continue;
        }
        let mut token = token.clone();
        if let Some(prev) = previous.take() {
            if ORDINAL_PATTERN.is_match(&token) && next.is_none_or(|n| TYPES_PATTERN.is_match(n)) {
                // Space removed to maximize chances to get a hit.
                token = prev.with_value(format!("{}{}", prev.value, token.value));
            } else {
                out.push(prev);
            }
        }
        out.push(token);
    }
    out
}

/// "3bis" → "3b".
pub fn fold_ordinal(token: Token) -> Token {
    let starts_with_digit = token.chars().next().is_some_and(|c| c.is_ascii_digit());
    if !starts_with_digit || is_digit(&token) {
        return token;
    }
    let Some(caps) = FOLD_PATTERN.captures(&token) else {
        return token;
    };
    let number = &caps[1];
    let ordinal = &caps[2];
    let lowered = ordinal.to_lowercase();
    let folded = FOLD
        .iter()
        .find(|(k, _)| *k == lowered)
        .map(|(_, v)| *v)
        .unwrap_or(ordinal);
    token.with_value(format!("{number}{folded}"))
}

/// The first number, at the start or right before a street type.
pub fn flag_housenumber(mut tokens: Vec<Token>) -> Vec<Token> {
    let mut found = false;
    for i in 0..tokens.len() {
        let next_is_type = tokens.get(i + 1).is_some_and(|n| TYPES_PATTERN.is_match(n));
        let token = &mut tokens[i];
        if (token.is_first || next_is_type) && NUMBER_PATTERN.is_match(token) && !found {
            token.kind = Some(TokenKind::Housenumber);
            found = true;
        }
    }
    tokens
}

/// Every combination of name, postcode, city and housenumber, so the query
/// can match whichever the user typed. Each city of the list counts, so an
/// address of a merged municipality is found under its former name too.
pub fn make_labels(doc: &Document, housenumber: Option<&str>) -> Vec<String> {
    let housenumber = housenumber.filter(|h| !h.is_empty());
    let add = |labels: &mut Vec<String>, label: String| {
        if let Some(hn) = housenumber {
            labels.insert(0, label.clone());
            labels.insert(0, format!("{hn} {label}"));
        } else {
            labels.insert(0, label);
        }
    };
    // A missing key reads as [""] (falsy), an empty list as [None].
    let cities: Vec<Option<String>> = match doc.get("city") {
        Some(serde_json::Value::Array(a)) if a.is_empty() => vec![None],
        _ => raw_values(doc, "city")
            .iter()
            .map(|v| truthy(v).then(|| crate::doc::py_str(v)))
            .collect(),
    };
    let postcode = first_str(doc, "postcode");
    let is_municipality = doc.get("type").and_then(|v| v.as_str()) == Some("municipality");
    let mut result = Vec::new();
    for name in str_values(doc, "name") {
        for city in &cities {
            let mut labels = Vec::new();
            let mut label = name.clone();
            if let Some(postcode) = postcode.as_deref()
                && is_municipality
            {
                add(&mut labels, format!("{label} {postcode}"));
                add(&mut labels, format!("{postcode} {label}"));
            }
            add(&mut labels, label.clone());
            if let Some(city) = city.as_deref()
                && city != label
            {
                add(&mut labels, format!("{label} {city}"));
                if let Some(postcode) = postcode.as_deref() {
                    label = format!("{label} {postcode}");
                    add(&mut labels, label.clone());
                    label = format!("{label} {city}");
                    add(&mut labels, label.clone());
                }
            }
            result.extend(labels);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(tokens: &[Token]) -> Vec<&str> {
        tokens.iter().map(|t| t.value.as_str()).collect()
    }

    #[test]
    fn glues_and_folds_ordinals() {
        let tokens = crate::text::tokenize("7 bis rue du faubourg");
        let tokens: Vec<Token> = glue_ordinal(tokens).into_iter().map(fold_ordinal).collect();
        assert_eq!(values(&tokens), ["7b", "rue", "du", "faubourg"]);
        let tokens = flag_housenumber(tokens);
        assert!(tokens[0].is_housenumber());
    }

    #[test]
    fn extracts_address() {
        assert_eq!(
            extract_address("xyz ets batiment b 22 rue des fleurs 59350 lille cedex 23"),
            "22 rue des fleurs 59350 lille cedex 23"
        );
    }

    #[test]
    fn cleans_query() {
        // Before a cedex, addok keeps only the department of the postcode.
        assert_eq!(
            clean_query("bp 12 22 rue des fleurs 59350 lille cedex 23"),
            "22 rue des fleurs 59 lille"
        );
    }
}
