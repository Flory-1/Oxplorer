//! Fuzzy matching for the command palette: the query's characters must
//! appear in order; consecutive runs and word starts score higher.

/// Score `text` against `query` (higher is better), or `None` if it doesn't match.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.chars().collect();
    let mut qi = 0;
    let mut score = 0;
    let mut prev_match: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        if qi == query.len() {
            break;
        }
        if c.to_lowercase().eq(std::iter::once(query[qi])) {
            let word_start = i == 0 || !chars[i - 1].is_alphanumeric();
            score += 1;
            if word_start {
                score += 6;
            }
            if prev_match == Some(i.wrapping_sub(1)) {
                score += 10;
            }
            prev_match = Some(i);
            qi += 1;
        }
    }
    if qi < query.len() {
        return None;
    }
    // Prefer shorter texts when everything else is equal.
    Some(score * 100 - chars.len() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_in_order() {
        assert!(score("nf", "New folder").is_some());
        assert!(score("fn", "New folder").is_none());
        assert!(score("", "anything").is_some());
        assert!(score("NEW FOL", "New folder").is_some());
    }

    #[test]
    fn ranks_word_starts_and_runs_higher() {
        let a = score("tg", "Toggle grid").unwrap();
        let b = score("tg", "Settings").unwrap();
        assert!(a > b, "{a} vs {b}");
        let run = score("sett", "Settings").unwrap();
        let spread = score("sett", "Show extension of tabs").unwrap();
        assert!(run > spread);
    }
}
