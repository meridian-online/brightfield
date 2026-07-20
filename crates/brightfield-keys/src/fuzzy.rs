//! A tiny dependency-free fuzzy subsequence scorer, shared by the palette filter
//! (verbs) and focus-jump (component paths). No allocation on the hot path.

/// Score `text` against `query` as a case-insensitive subsequence match.
///
/// Returns `None` when `query` is not a subsequence of `text`. An empty query
/// matches everything with score `0` (so an empty palette query keeps every
/// candidate, to be ordered by frequency/recency downstream). Higher scores are
/// better: contiguous runs and word-start (after a non-alphanumeric boundary)
/// matches are rewarded, later start positions are mildly penalised.
#[must_use]
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let t: Vec<char> = text.chars().flat_map(char::to_lowercase).collect();

    let mut qi = 0usize;
    let mut score = 0i32;
    let mut run = 0i32; // consecutive-match run length
    let mut first_match: Option<usize> = None;

    for (ti, &tc) in t.iter().enumerate() {
        if qi < q.len() && tc == q[qi] {
            if first_match.is_none() {
                first_match = Some(ti);
            }
            // Word-boundary bonus: a match at the start, or just after a
            // non-alphanumeric char, reads as an initialism hit.
            let at_boundary = ti == 0 || t.get(ti - 1).is_some_and(|c| !c.is_alphanumeric());
            score += 1 + run + i32::from(at_boundary) * 3;
            run += 1;
            qi += 1;
        } else {
            run = 0;
        }
    }

    if qi == q.len() {
        // Penalise a late first match so an early hit ranks above a buried one.
        let lead = first_match.unwrap_or(0) as i32;
        Some(score - lead / 4)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kbg_fuzzy_empty_query_matches_all() {
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    #[test]
    fn kbg_fuzzy_subsequence_matches_and_non_subsequence_misses() {
        assert!(fuzzy_score("col", "cycle-colour-scheme").is_some()); // subsequence
        assert!(fuzzy_score("clr", "cycle-colour-scheme").is_some()); // c…l…r is a subsequence too
        assert!(fuzzy_score("qz", "cycle-colour-scheme").is_none()); // no q, no z
        assert!(fuzzy_score("xyz", "cycle-colour-scheme").is_none());
    }

    #[test]
    fn kbg_fuzzy_case_insensitive() {
        assert!(fuzzy_score("COL", "cycle-colour-scheme").is_some());
    }

    #[test]
    fn kbg_fuzzy_word_boundary_outranks_buried() {
        // "cc" hits two word-starts (c@0, c@"colour"); "yl" is buried inside "cycle".
        let boundary = fuzzy_score("cc", "cycle-colour-scheme").unwrap();
        let buried = fuzzy_score("yl", "cycle-colour-scheme").unwrap();
        assert!(
            boundary > buried,
            "boundary {boundary} should beat buried {buried}"
        );
    }
}
