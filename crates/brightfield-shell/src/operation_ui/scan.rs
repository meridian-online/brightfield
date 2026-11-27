//! Reading SQL text a form holds, such as an order or a key, at its top
//! level: outside quotes and parentheses, where a comma ends a list item.

/// What a character of SQL text is part of, read left to right: a `"name"`
/// or a `'literal'` (a doubled quote inside one closes and opens it again,
/// which leaves it open), and how many parentheses are open around it.
#[derive(Default)]
pub(super) struct Scan {
    quote: Option<char>,
    depth: usize,
}

impl Scan {
    /// Take `c`, and say whether it stands at the top level: outside every
    /// quote and parenthesis, its own opening one included.
    pub(super) fn top(&mut self, c: char) -> bool {
        match (self.quote, c) {
            (Some(q), _) => {
                if c == q {
                    self.quote = None;
                }
                false
            }
            (None, '"' | '\'') => {
                self.quote = Some(c);
                false
            }
            (None, '(') => {
                self.depth += 1;
                false
            }
            (None, ')') => {
                self.depth = self.depth.saturating_sub(1);
                false
            }
            (None, _) => self.depth == 0,
        }
    }
}

/// The items of a comma-separated list, split at commas at the top level only
/// and trimmed, without an empty one: `"price, usd", round(x, 1)` is two items.
pub(super) fn top_level_items(list: &str) -> Vec<&str> {
    let mut scan = Scan::default();
    let mut items = Vec::new();
    let mut start = 0;
    for (at, c) in list.char_indices() {
        if scan.top(c) && c == ',' {
            items.push(&list[start..at]);
            start = at + c.len_utf8();
        }
    }
    items.push(&list[start..]);
    items
        .into_iter()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_list_splits_only_at_commas_outside_quotes_and_parentheses() {
        assert_eq!(
            top_level_items(r#" "price, usd", round(x, 1) desc, 'a,b' ,, city"#),
            vec![r#""price, usd""#, "round(x, 1) desc", "'a,b'", "city"]
        );
        assert!(top_level_items(" , ").is_empty());
    }
}
