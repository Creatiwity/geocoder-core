pub mod builder;
pub mod format;
pub mod store;

pub use builder::IndexBuilder;
pub use format::Meta;
pub use store::Index;

use crate::types::DocId;

/// Sort key reproducing Redis member order for `d|<n>` keys, where `n` is
/// the 1-based import order: compared as text, not as numbers.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TieKey([u8; 10]);

pub(crate) fn tie_break(doc: DocId) -> TieKey {
    let mut n = doc as u64 + 1;
    let mut digits = [0u8; 10];
    let mut len = 0;
    while n > 0 {
        digits[len] = b'0' + (n % 10) as u8;
        n /= 10;
        len += 1;
    }
    digits[..len].reverse();
    // NUL padding sorts before any digit: "10" > "1" and "2" > "10", like strings.
    TieKey(digits)
}

#[cfg(test)]
mod tests {
    use super::tie_break;

    #[test]
    fn orders_like_strings() {
        // "d|2" > "d|10" > "d|1"
        assert!(tie_break(1) > tie_break(9));
        assert!(tie_break(9) > tie_break(0));
    }
}
