/// Letters around each key of a keyboard, used to limit fuzzy substitutions.
pub type KeyMap = &'static [(char, &'static str)];

/// addok's default `FUZZY_KEY_MAP` (French AZERTY).
pub const AZERTY: KeyMap = &[
    ('a', "ezqop"),
    ('z', "aqse"),
    ('e', "azsdryu"),
    ('r', "edft"),
    ('t', "rfgy"),
    ('y', "teghu"),
    ('u', "yehji"),
    ('i', "ujko"),
    ('o', "iaklp"),
    ('p', "oalm"),
    ('q', "azsw"),
    ('s', "qzedxw"),
    ('d', "serfcx"),
    ('f', "drtgvc"),
    ('g', "ftyhbv"),
    ('h', "gyujnb"),
    ('j', "huikn"),
    ('k', "jil"),
    ('l', "kom"),
    ('m', "lpu"),
    ('w', "qsx"),
    ('x', "wsdc"),
    ('c', "xdfvio"),
    ('v', "cfgb"),
    ('b', "vghn"),
    ('n', "bhj"),
];

/// `addok.fuzzy.make_fuzzy` (1.1): inversions, substitutions (limited to
/// neighbouring keys when a key map is given), insertions, then removals for
/// words longer than 3, deduplicated keeping that order (it ranks the
/// candidates).
pub fn make_fuzzy(word: &str, key_map: Option<KeyMap>) -> Vec<String> {
    const LETTERS: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    let chars: Vec<char> = word.chars().collect();
    let mut neighbors: Vec<String> = Vec::new();
    for i in 0..chars.len().saturating_sub(1) {
        let mut n = chars.clone();
        n.swap(i, i + 1);
        neighbors.push(n.iter().collect());
    }
    match key_map {
        Some(map) => {
            for i in 0..chars.len() {
                if let Some((_, around)) = map.iter().find(|(k, _)| *k == chars[i]) {
                    for letter in around.chars().filter(|&l| l != chars[i]) {
                        let mut n = chars.clone();
                        n[i] = letter;
                        neighbors.push(n.iter().collect());
                    }
                }
            }
        }
        None => {
            for &letter in LETTERS {
                let letter = letter as char;
                for i in 0..chars.len() {
                    if chars[i] != letter {
                        let mut n = chars.clone();
                        n[i] = letter;
                        neighbors.push(n.iter().collect());
                    }
                }
            }
        }
    }
    for &letter in LETTERS {
        for i in 0..=chars.len() {
            let mut n = chars.clone();
            n.insert(i, letter as char);
            neighbors.push(n.iter().collect());
        }
    }
    if chars.len() > 3 {
        for i in 0..chars.len() {
            let mut n = chars.clone();
            n.remove(i);
            neighbors.push(n.iter().collect());
        }
    }
    let mut seen = std::collections::HashSet::new();
    neighbors.retain(|n| seen.insert(n.clone()));
    neighbors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_limited_substitutions() {
        let all = make_fuzzy("rue", None);
        let azerty = make_fuzzy("rue", Some(AZERTY));
        assert!(azerty.len() < all.len());
        assert_eq!(azerty[0], "ure");
        // "r" → "e" is a neighbour key, "r" → "p" is not.
        assert!(azerty.contains(&"eue".to_string()));
        assert!(!azerty.contains(&"pue".to_string()));
        // Deduplicated: "ruee" comes from two insertions.
        assert_eq!(azerty.iter().filter(|n| *n == "ruee").count(), 1);
    }
}
