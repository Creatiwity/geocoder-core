/// `addok.helpers.text.Token`: a normalized word with the metadata the
/// pipeline and the collectors rely on.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub value: String,
    /// Positions, one per level of subdivision (a synonym expanding into
    /// several words appends the sub-position).
    pub positions: Vec<usize>,
    pub is_first: bool,
    pub is_last: bool,
    pub kind: Option<TokenKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Housenumber,
}

impl Token {
    pub fn new(value: String, position: usize) -> Self {
        Self {
            value,
            positions: vec![position],
            is_first: position == 0,
            is_last: false,
            kind: None,
        }
    }

    /// `Token.update`: a new value keeping the metadata. `is_first` is
    /// recomputed from the first position, like addok does.
    pub fn with_value(&self, value: String) -> Self {
        Self {
            value,
            ..self.clone()
        }
    }

    /// `Token.update(value, position=p)`: the sub-position is appended.
    pub fn with_value_at(&self, value: String, sub_position: usize) -> Self {
        let mut token = self.with_value(value);
        token.positions.push(sub_position);
        token.is_first = token.positions[0] == 0;
        token
    }

    pub fn is_housenumber(&self) -> bool {
        self.kind == Some(TokenKind::Housenumber)
    }
}

impl std::ops::Deref for Token {
    type Target = str;
    fn deref(&self) -> &str {
        &self.value
    }
}
