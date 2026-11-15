use std::collections::HashMap;
use std::sync::Mutex;

use fancy_regex::Regex;
use once_cell::sync::Lazy;

/// `addok_fr.utils.RULES` from addok-fr 1.1.0, the version production runs.
/// Applied in order, each as a global substitution (Python's `re.sub`).
static RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    let raw: &[(&str, &str)] = &[
        ("(?<=a)(mp|nd|nt)s?$", "n"),             // champ(s) > chan
        (r"([aeiouy])mp(?=[^aeiouyr])", "${1}n"), // champvallon -> chanvalon
        (r"ngt(?=[aeiouy])", "nt"),               // vingtieme > vintieme
        (r"ngt", "n"),                            // vingt > vin
        ("((?<=[^g])g|^g)(?=[eyi])", "j"),
        ("(?<=g)u(?=[aeio])", ""),
        (r"(?<=ei)gn([aeiouy])", "ni${1}"), // seigneur -> senieur
        (r"je([aeiouy])", "j${1}"),         // georges -> jorj
        ("c(?=[^hieyw])", "k"),
        (r"anc$", "an"), // blanc -> blan
        // Final "c", "ch", but not "sch" and not "10c".
        ("((?<=[^s])ch|(?<=[^0-9])c)$", "k"),
        ("(?<=[aeiouy])s(?=[aeiouy])", "z"),
        ("((?<=[^0-9])q|^q)u?", "k"),
        ("cc(?=[ie])", "s"),
        ("ck", "k"),
        ("ph", "f"),
        ("th$", "te"),
        ("(?<=[^sc0-9])h", ""),
        // Python's `^h(?=.)+`: a repeated lookahead is the lookahead itself.
        ("^h(?=.)", ""),
        ("sc", "s"),
        ("sh", "ch"),
        ("((?<=[^0-9])w|^w)", "v"),
        ("c(?=[eiy])", "s"),
        (r"((?<=[^0-9])y|^y)", "i"),
        ("esn", "en"),
        // The captured space is dropped, like Python's replacement "aim".
        (r"eim( |$)", "aim"), // pforzheim -> pforzaim
        (r"(ae|ei)(?=\w)", "e"),
        (r"oeufs( |$)", "eu"),
        (r"oeu?(?=\w)", "eu"),
        ("(?<=[^0-9])s$", ""),
        ("(?<=u)l?x$", ""), // eaux, eux, aux, aulx
        ("(?<=u)lt$", "t"),
        ("(?<=[a-z])[dg]$", ""),
        ("(?<=[^es0-9])t$", ""),
        ("(?<=[aeiou])(m)(?=[pbgft])", "n"), // impossible -> inpossible
        // Final "e" after at least two letters.
        ("(?<=[a-z]{2})(e$)", ""),
        (r"(?<=[aeiouy])n[dt](?=[^aeiouyr])", "n"), // montbon -> monbon
        (r"([a-z])\1+", "${1}"),                    // duplicate letters
    ];
    raw.iter()
        .map(|(pattern, repl)| {
            let re = Regex::new(pattern).unwrap_or_else(|e| panic!("phonemicize {pattern:?}: {e}"));
            (re, *repl)
        })
        .collect()
});

/// Indexing sees the same few hundred thousand words millions of times.
static CACHE: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Very lite French phonemicization: drop every letter that is not
/// significant.
pub fn phonemicize(s: &str) -> String {
    if let Some(hit) = CACHE.lock().unwrap().get(s) {
        return hit.clone();
    }
    let mut out = s.to_string();
    for (re, repl) in RULES.iter() {
        out = re.replace_all(&out, *repl).into_owned();
    }
    CACHE.lock().unwrap().insert(s.to_string(), out.clone());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Expected values produced by addok-fr 1.1.0 itself (scripts/addok_oracle.py).
    #[test]
    fn matches_addok_fr() {
        let cases = [
            ("cergy", "serji"),
            ("conflans", "konflan"),
            ("dunkerque", "dunkerk"),
            ("wardrecques", "vardrek"),
            ("sault", "sau"),
            ("vaux", "vau"),
            ("guyancourt", "guiankour"),
            ("hazebrouck", "azebrouk"),
            ("vallee", "vale"),
            ("champs", "chan"),
            ("vingt", "vin"),
            ("vingtieme", "vintiem"),
            ("blanc", "blan"),
            ("georges", "jorj"),
            ("seigneur", "senieur"),
            ("pforzheim", "pforzaim"),
            ("montbon", "monbon"),
            ("10a", "10a"),
        ];
        for (input, expected) in cases {
            assert_eq!(phonemicize(input), expected, "phonemicize({input:?})");
        }
    }
}
