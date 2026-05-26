use unidecode::unidecode;

/// Transliterate unicode to ASCII, lowercase, replace non-alphanumeric with spaces.
/// Mirrors addok's: re.sub(r'[^a-z\d]', ' ', unidecode(s.lower()))
pub fn normalize(input: &str) -> String {
    unidecode(input)
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_accents() {
        assert_eq!(normalize("Île-de-France"), "ile de france");
    }

    #[test]
    fn test_uppercase() {
        assert_eq!(normalize("PARIS"), "paris");
    }

    #[test]
    fn test_special_chars() {
        assert_eq!(normalize("Saint-Étienne"), "saint etienne");
    }
}
