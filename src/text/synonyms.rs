use once_cell::sync::Lazy;
use std::collections::HashMap;

/// French address abbreviation map, ported from addok-fr/addok_fr/resources/synonyms.txt.
/// Format: abbreviation(s) → canonical form (one-directional, same as addok).
/// Keys are the abbreviated forms; values are the canonical forms to look up in the index.
static SYNONYMS: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    // (abbreviated_form, canonical_form) — multiple abbreviations for the same canonical
    // form appear as separate entries. All forms are pre-normalized (lowercase ASCII).
    const ENTRIES: &[(&str, &str)] = &[
        // Street types — from addok-fr synonyms.txt
        ("all", "allee"),
        ("al", "allee"),
        ("alle", "allee"),
        ("av", "avenue"),
        ("ave", "avenue"),
        ("aven", "avenue"),
        ("bd", "boulevard"),
        ("bld", "boulevard"),
        ("bou", "boulevard"),
        ("bould", "boulevard"),
        ("bvd", "boulevard"),
        ("boul", "boulevard"),
        ("ber", "berge"),
        ("bcle", "boucle"),
        ("brtl", "bretelle"),
        ("bsn", "bassin"),
        ("caref", "carrefour"),
        ("car", "carriere"),
        ("cd", "chemin departemental"),
        ("cf", "chemin forestier"),
        ("ch", "chemin"),
        ("che", "chemin"),
        ("chem", "chemin"),
        ("cr", "chemin rural"),
        ("cv", "chemin vicinal"),
        ("chl", "chalet"),
        ("chp", "champ"),
        ("chps", "champs"),
        ("cha", "chaussee"),
        ("chs", "chaussee"),
        ("chse", "chaussee"),
        ("chss", "chaussee"),
        ("cht", "chateau"),
        ("chev", "chevalier"),
        ("ci", "cite"),
        ("cit", "cite"),
        ("clo", "clos"),
        ("clr", "couloir"),
        ("col", "colonel"),
        ("cor", "corniche"),
        ("crnc", "corniche"),
        ("cpg", "camping"),
        ("cpt", "capitaine"),
        ("cou", "cour"),
        ("crx", "croix"),
        ("ctre", "centre"),
        ("dept", "departement"),
        ("devi", "deviation"),
        ("dig", "digue"),
        ("dom", "domaine"),
        ("dr", "docteur"),
        ("doct", "docteur"),
        ("dra", "draille"),
        ("dre", "dreve"),
        ("dsc", "descente"),
        ("ecl", "ecluse"),
        ("embr", "embranchement"),
        ("enc", "enclos"),
        ("enf", "enfants"),
        ("enfts", "enfants"),
        ("entr", "entree"),
        ("espa", "espace"),
        ("espl", "esplanade"),
        ("etng", "etang"),
        ("fac", "facade"),
        ("fg", "faubourg"),
        ("fbg", "faubourg"),
        ("fbourg", "faubourg"),
        ("fon", "fontaine"),
        ("font", "fontaine"),
        ("for", "foret"),
        ("frm", "ferme"),
        ("gale", "galerie"),
        ("gbd", "grand boulevard"),
        ("gde", "grande"),
        ("gdes", "grandes"),
        ("gds", "grands"),
        ("gen", "general"),
        ("gal", "general"),
        ("gl", "general"),
        ("gir", "giratoire"),
        ("gpl", "grand place"),
        ("gr", "grande rue"),
        ("grp", "groupe"),
        ("grpe", "groupe"),
        ("hab", "habitation"),
        ("habit", "habitation"),
        ("ham", "hameau"),
        ("hle", "halle"),
        ("hlg", "halage"),
        ("hop", "hopital"),
        ("hosp", "hopital"),
        ("ht", "haut"),
        ("hte", "haute"),
        ("htes", "hautes"),
        ("hts", "hauts"),
        ("imp", "impasse"),
        ("jard", "jardin"),
        ("jar", "jardin"),
        ("jte", "jetee"),
        ("lnt", "lieutenant"),
        ("lnant", "lieutenant"),
        ("lieut", "lieutenant"),
        ("lt", "lieutenant"),
        ("lot", "lotissement"),
        ("lotis", "lotissement"),
        ("lotiss", "lotissement"),
        ("lyc", "lycee"),
        ("mal", "marechal"),
        ("ml", "marechal"),
        ("mais", "maison"),
        ("march", "marche"),
        ("mln", "moulin"),
        ("mme", "madame"),
        ("mgr", "monseigneur"),
        ("mqs", "marquis"),
        ("mqse", "marquise"),
        ("mrn", "marina"),
        ("mt", "mont"),
        ("mte", "montee"),
        ("mtgne", "montagne"),
        ("nd", "notre dame"),
        ("nouv", "nouveau"),
        ("nte", "nouvelle route"),
        ("nve", "neuve"),
        ("pae", "petite avenue"),
        ("pass", "passage"),
        ("pge", "passage"),
        ("psge", "passage"),
        ("pch", "petit chemin"),
        ("pdt", "president"),
        ("periph", "peripherique"),
        ("pkg", "parking"),
        ("pl", "place"),
        ("pla", "place"),
        ("plac", "place"),
        ("ple", "passerelle"),
        ("pln", "plaine"),
        ("plt", "plateau"),
        ("pnt", "pointe"),
        ("porq", "portique"),
        ("pr", "pere"),
        ("pref", "prefecture"),
        ("prof", "professeur"),
        ("prom", "promenade"),
        ("prm", "promenade"),
        ("pta", "petite allee"),
        ("pte", "porte"),
        ("ptr", "petite rue"),
        ("ptte", "placette"),
        ("pvr", "parvis"),
        ("pai", "parvis"),
        ("prv", "parvis"),
        ("qu", "quai"),
        ("q", "quai"),
        ("qrt", "quartier"),
        ("qua", "quartier"),
        ("quart", "quartier"),
        ("r", "rue"),
        ("ran", "rangee"),
        ("rav", "ravin"),
        ("rd", "route departementale"),
        ("rdl", "raidillon"),
        ("regt", "regiment"),
        ("renc", "rencontre"),
        ("rep", "republique"),
        ("res", "residence"),
        ("rfg", "refuge"),
        ("rle", "ruelle"),
        ("ruel", "ruelle"),
        ("rul", "ruelle"),
        ("rn", "route nationale"),
        ("rpe", "rampe"),
        ("rpt", "rond point"),
        ("rdpoint", "rond point"),
        ("rop", "rond point"),
        ("rdp", "rond point"),
        ("rondpoint", "rond point"),
        ("rdpt", "rond point"),
        ("rtd", "rotonde"),
        ("rte", "route"),
        ("rou", "route"),
        ("ruet", "ruette"),
        ("ruis", "ruisseau"),
        ("rult", "ruellette"),
        ("rve", "ravine"),
        ("sen", "sentier"),
        ("sent", "sentier"),
        ("sgt", "sergent"),
        ("sq", "square"),
        ("squ", "square"),
        ("ss", "sous"),
        ("std", "stade"),
        ("stde", "stade"),
        ("stg", "stage"),
        ("tpl", "terre plein"),
        ("trab", "traboule"),
        ("trn", "terrain"),
        ("trt", "tertre"),
        ("tsse", "terrasse"),
        ("ter", "terrasse"),
        ("tun", "tunnel"),
        ("tunn", "tunnel"),
        ("valee", "vallee"),
        ("vall", "vallon"),
        ("vc", "voie communale"),
        ("vche", "vieux chemin"),
        ("vch", "vieux chemin"),
        ("ven", "venelle"),
        ("vge", "village"),
        ("viad", "viaduc"),
        ("vil", "ville"),
        ("vla", "villa"),
        ("vte", "vieille route"),
        ("za", "zone artisanale"),
        ("zac", "zone amenagement concerte"),
        ("zi", "zone industrielle"),
        ("zup", "zone urbaniser en priorite"),
        // Proper names (saints, titles)
        ("st", "saint"),
        ("ste", "sainte"),
        ("sts", "saints"),
        ("stes", "saintes"),
        // Institutions / ranks
        ("gal", "general"),
        ("mal", "marechal"),
        ("pdt", "president"),
        // Ordinals (most common ones)
        ("1er", "premier"),
        ("1e", "premier"),
        ("ier", "premier"),
        ("1ere", "premiere"),
        ("1re", "premiere"),
    ];

    let mut map = HashMap::new();
    for &(abbrev, canonical) in ENTRIES {
        map.insert(abbrev, canonical);
    }
    map
});

/// Returns the canonical form for a normalized (lowercase, ASCII) abbreviated token,
/// or None if no synonym exists. Mirrors addok-fr's SYNONYMS dict lookup.
pub fn synonyms_for(token: &str) -> Option<&'static str> {
    SYNONYMS.get(token).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abbrev_to_canonical() {
        assert_eq!(synonyms_for("av"), Some("avenue"));
        assert_eq!(synonyms_for("bd"), Some("boulevard"));
        assert_eq!(synonyms_for("r"), Some("rue"));
        assert_eq!(synonyms_for("st"), Some("saint"));
        assert_eq!(synonyms_for("fg"), Some("faubourg"));
        assert_eq!(synonyms_for("imp"), Some("impasse"));
        assert_eq!(synonyms_for("pl"), Some("place"));
    }

    #[test]
    fn test_canonical_has_no_synonym() {
        // Canonical forms are not keys in the map (one-directional)
        assert_eq!(synonyms_for("avenue"), None);
        assert_eq!(synonyms_for("boulevard"), None);
        assert_eq!(synonyms_for("rue"), None);
        assert_eq!(synonyms_for("saint"), None);
    }

    #[test]
    fn test_unknown() {
        assert_eq!(synonyms_for("paix"), None);
        assert_eq!(synonyms_for("paris"), None);
    }
}
