use fancy_regex::Regex; // crate: fancy-regex
use once_cell::sync::Lazy;

/// French phonemicization rules, ported from addok-fr/addok_fr/utils.py.
/// Applied in order, each as a global replace, on a single normalized token.
static RULES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    let raw: &[(&str, &str)] = &[
        // 1: champ(s) → chan  (a + mp/nd/nt + optional s at end)
        (r"(?<=a)(mp|nd|nt)s?$", "n"),
        // 2: champvallon → chanvalon  (vowel + mp + consonant)
        (r"([aeiouy])mp(?=[^aeiouyr])", "${1}n"),
        // 3: vingtième → vintième  (ngt before vowel)
        (r"ngt(?=[aeiouy])", "nt"),
        // 4: vingt → vin
        (r"ngt", "n"),
        // 5: ge/gy/gi → je/jy/ji  (g before e/y/i, not after g)
        (r"((?<=[^g])g|^g)(?=[eyi])", "j"),
        // 6: gue/gua/gui/guo → ge/ga/gi/go  (remove u after g before vowel)
        (r"(?<=g)u(?=[aeio])", ""),
        // 7: seigneur → senieur  (gn after ei + vowel)
        (r"(?<=ei)gn([aeiouy])", "ni${1}"),
        // 8: jean → jan  (je before vowel)
        (r"je([aeiouy])", "j${1}"),
        // 9: c before consonants → k  (not before h/i/e/y/w)
        (r"c(?=[^hieyw])", "k"),
        // 10: blanc → blan  (anc at end)
        (r"anc$", "an"),
        // 11: final ch (not sch) or final c (not after digit) → k
        (r"((?<=[^s])ch|(?<=[^0-9])c)$", "k"),
        // 12: intervocalic s → z  (maison → maizon)
        (r"(?<=[aeiouy])s(?=[aeiouy])", "z"),
        // 13: qu/q → k  (not after digit)
        (r"((?<=[^0-9])q|^q)u?", "k"),
        // 14: cc before i/e → s
        (r"cc(?=[ie])", "s"),
        // 15: ck → k
        (r"ck", "k"),
        // 16: ph → f
        (r"ph", "f"),
        // 17: final th → te
        (r"th$", "te"),
        // 18: silent h (not after s, c, or digit)
        (r"(?<=[^sc0-9])h", ""),
        // 19: initial h when word has more than one char
        (r"^h(?=.)", ""),
        // 20: sc → s
        (r"sc", "s"),
        // 21: sh → ch
        (r"sh", "ch"),
        // 22: w → v  (not after digit)
        (r"((?<=[^0-9])w|^w)", "v"),
        // 23: c before e/i/y → s
        (r"c(?=[eiy])", "s"),
        // 24: y → i  (not after digit)
        (r"((?<=[^0-9])y|^y)", "i"),
        // 25: esn → en
        (r"esn", "en"),
        // 26: eim at word end or before space → aim  (pforzheim → pforzaim)
        (r"eim( |$)", "aim$1"),
        // 27: ae/ei before word char → e
        (r"(ae|ei)(?=\w)", "e"),
        // 28: oeufs at word end or before space → eu  (special plural)
        (r"oeufs( |$)", "eu$1"),
        // 29: oe/oeu before word char → eu
        (r"oeu?(?=\w)", "eu"),
        // 30: final s (not after digit) → remove
        (r"(?<=[^0-9])s$", ""),
        // 31: final ux/ulx after u → remove  (eaux, eux, aux, aulx)
        (r"(?<=u)l?x$", ""),
        // 32: final ult after u → t  (sault → sau → rule 31)
        (r"(?<=u)lt$", "t"),
        // 33: final d or g after letter → remove  (grand → gran)
        (r"(?<=[a-z])[dg]$", ""),
        // 34: final t (not after e/s/digit) → remove  (chat → cha)
        (r"(?<=[^es0-9])t$", ""),
        // 35: m → n before labial/dental consonants  (impossible → inpossible)
        (r"(?<=[aeiou])m(?=[pbgft])", "n"),
        // 36: silent final e (after at least 2 letters)  (rue → ru)
        (r"(?<=[a-z]{2})e$", ""),
        // 37: nd/nt after vowel before non-vowel → n  (montbon → monbon)
        (r"(?<=[aeiouy])n[dt](?=[^aeiouyr])", "n"),
        // 38: deduplicate consecutive identical letters  (valllee → vale)
        (r"([a-z])\1+", "$1"),
    ];
    raw.iter()
        .map(|(pattern, repl)| {
            (
                Regex::new(pattern).unwrap_or_else(|e| {
                    panic!("phonemicize: invalid regex {pattern:?}: {e}")
                }),
                *repl,
            )
        })
        .collect()
});

/// Apply French phonemicization to a single normalized token.
/// Mirrors addok-fr's phonemicize() from addok_fr/utils.py.
/// Input must be lowercase ASCII (i.e., already passed through normalize()).
pub fn phonemicize(token: &str) -> String {
    let mut s = token.to_string();
    for (re, repl) in RULES.iter() {
        s = re.replace_all(&s, *repl).into_owned();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // Representative subset of addok-fr/tests/test_utils.py
    macro_rules! ph {
        ($input:expr, $expected:expr) => {
            assert_eq!(
                phonemicize($input),
                $expected,
                "phonemicize({:?})",
                $input
            );
        };
    }

    #[test]
    fn test_phonemicize() {
        ph!("cergy", "serji");
        ph!("andresy", "andrezi");
        ph!("conflans", "konflan");
        ph!("watel", "vatel");
        ph!("dunkerque", "dunkerk");
        ph!("robecq", "robek");
        ph!("wardrecques", "vardrek");
        ph!("cabourg", "kabour");
        ph!("audinghen", "audingen");
        ph!("sault", "sau");
        ph!("vaux", "vau");
        ph!("guyancourt", "guiankour");
        ph!("abasset", "abaset");
        ph!("agenest", "ajenest");
        ph!("ableh", "abl");
        ph!("abilhous", "abilou");
        ph!("amfreville", "anfrevil");
        ph!("rimfort", "rinfor");
        ph!("abatesco", "abatesko");
        ph!("albiosc", "albiosk");
        ph!("desc", "desk");
        ph!("bricquebec", "brikebek");
        ph!("locmariaquer", "lokmariaker");
        ph!("ancetres", "ansetr");
        ph!("vicdessos", "vikdeso");
        ph!("acacias", "akasia");
        ph!("placis", "plasi");
        ph!("courcome", "kourkom");
        ph!("hazebrouck", "azebrouk");
        ph!("blotzheim", "blotzaim");
        ph!("plouhinec", "plouinek");
        ph!("hirschland", "irchlan");
        ph!("schlierbach", "chlierbak");
        ph!("boeufs", "beu");
        ph!("vallee", "vale");
        ph!("valllee", "vale");
        ph!("montgriffon", "mongrifon");
        ph!("montagne", "montagn");
        ph!("montage", "montaj");
        ph!("champs", "chan");
        ph!("grandchamps", "granchan");
        ph!("vingt", "vin");
        ph!("vingtieme", "vintiem");
        ph!("champvallon", "chanvalon");
        ph!("champol", "chanpol");
        ph!("montbon", "monbon");
        ph!("montgros", "mongro");
        ph!("blanc", "blan");
        ph!("montee", "monte");
        ph!("seigneur", "senieur");
        ph!("pforzheim", "pforzaim");
        ph!("georges", "jorj");
        ph!("gorges", "gorj");
        ph!("quimper", "kinper");
        ph!("h", "h");
        ph!("blanc", "blan");
        // digit+letter housenumber-style tokens must be unchanged
        ph!("10a", "10a");
        ph!("10s", "10s");
        ph!("1e", "1e");
    }
}
