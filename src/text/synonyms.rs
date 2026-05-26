use once_cell::sync::Lazy;
use std::collections::HashMap;

/// Bidirectional French address synonym map (normalized lowercase).
/// Built from AFNOR postal abbreviation standard used in BAN data.
/// Both directions: "avenue" → ["av"] and "av" → ["avenue"].
static SYNONYMS: Lazy<HashMap<&'static str, &'static [&'static str]>> = Lazy::new(|| {
    // (long_form, short_form) pairs — bidirectional
    const PAIRS: &[(&str, &str)] = &[
        // Street types
        ("allee", "all"),
        ("avenue", "av"),
        ("boulevard", "bd"),
        ("chemin", "chem"),
        ("chemins", "chem"),
        ("cheminement", "chem"),
        ("domaine", "doma"),
        ("domaines", "doma"),
        ("escalier", "esca"),
        ("escaliers", "esca"),
        ("esplanade", "espl"),
        ("faubourg", "faub"),
        ("hameau", "hame"),
        ("hameaux", "hame"),
        ("impasse", "imp"),
        ("impasses", "imp"),
        ("lieu dit", "ld"),
        ("lotissement", "loti"),
        ("lotissements", "loti"),
        ("passage", "pass"),
        ("place", "pl"),
        ("promenade", "prom"),
        ("quartier", "quar"),
        ("residence", "resi"),
        ("residences", "resi"),
        ("rond point", "rpt"),
        ("route", "rte"),
        ("ruelle", "ruel"),
        ("ruelles", "ruel"),
        ("sente", "sent"),
        ("sentier", "sent"),
        ("sentiers", "sent"),
        ("square", "sq"),
        ("traverse", "trav"),
        ("vallee", "vall"),
        ("venelle", "vene"),
        ("venelles", "vene"),
        ("villa", "vill"),
        ("villas", "vill"),
        ("village", "vlge"),
        ("voie", "voie"),
        // Proper names (very common in French addresses)
        ("saint", "st"),
        ("sainte", "ste"),
        ("saints", "sts"),
        ("saintes", "stes"),
        // Common adjectives in address names
        ("grand", "gd"),
        ("grande", "gde"),
        ("grands", "gds"),
        ("grandes", "gdes"),
        ("nouveau", "nouv"),
        ("nouvelle", "nouv"),
        ("vieux", "vx"),
        ("vieille", "viel"),
        ("vieilles", "viel"),
        ("petit", "pt"),
        ("petite", "pte"),
        ("petits", "pts"),
        ("petites", "ptes"),
        // Institutions
        ("docteur", "dr"),
        ("general", "gal"),
        ("lieutenant", "lt"),
        ("marechal", "mal"),
        ("president", "pdt"),
        ("professeur", "pr"),
        ("republique", "rep"),
    ];

    let mut map: HashMap<&'static str, &'static [&'static str]> = HashMap::new();

    // Leak vecs to get 'static slices for the map values
    for &(long, short) in PAIRS {
        let long_syns: &'static [&'static str] = Box::leak(Box::new([short]));
        let short_syns: &'static [&'static str] = Box::leak(Box::new([long]));
        map.insert(long, long_syns);
        map.insert(short, short_syns);
    }

    map
});

/// Returns synonym forms for a normalized (lowercase, ascii) token.
/// Empty slice if no synonym exists.
pub fn synonyms_for(token: &str) -> &'static [&'static str] {
    SYNONYMS.get(token).copied().unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_long_to_short() {
        assert!(synonyms_for("avenue").contains(&"av"));
    }

    #[test]
    fn test_short_to_long() {
        assert!(synonyms_for("av").contains(&"avenue"));
    }

    #[test]
    fn test_saint() {
        assert!(synonyms_for("saint").contains(&"st"));
        assert!(synonyms_for("st").contains(&"saint"));
    }

    #[test]
    fn test_unknown() {
        assert!(synonyms_for("paix").is_empty());
    }
}
