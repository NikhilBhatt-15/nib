//! Fuzzy matching for Quick Open and the command palette, in the style of
//! Sublime Text: every query character must appear in order; consecutive
//! runs, word starts and matches in the file name score higher.

/// Score and matched char indices, or None when `query` doesn't match.
pub fn score(query: &str, target: &str) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let t: Vec<char> = target.chars().collect();
    let tl: Vec<char> = t.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let q: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect();
    if q.is_empty() {
        return Some((0, Vec::new()));
    }
    // Start of the final path component: matches there are worth more.
    let name_start = t.iter().rposition(|&c| c == '\\' || c == '/').map_or(0, |i| i + 1);

    // Greedy match from the right-most viable start, then a forward pass that
    // prefers word starts. Good enough for interactive ranking.
    let mut best: Option<(i32, Vec<usize>)> = None;
    let starts: Vec<usize> = (0..tl.len()).filter(|&i| tl[i] == q[0]).collect();
    for &s in starts.iter().take(24) {
        let mut idx = Vec::with_capacity(q.len());
        let mut ti = s;
        let mut ok = true;
        for (qi, &qc) in q.iter().enumerate() {
            if qi == 0 {
                idx.push(s);
                ti = s + 1;
                continue;
            }
            // Prefer the next word-start occurrence if it comes soon.
            let mut found = None;
            let mut first_any = None;
            for j in ti..tl.len() {
                if tl[j] == qc {
                    if first_any.is_none() {
                        first_any = Some(j);
                    }
                    if j == ti || is_word_start(&t, j) {
                        found = Some(j);
                        break;
                    }
                }
            }
            match found.or(first_any) {
                Some(j) => {
                    idx.push(j);
                    ti = j + 1;
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let mut sc = 0i32;
        for (k, &i) in idx.iter().enumerate() {
            sc += 10;
            if is_word_start(&t, i) {
                sc += 18;
            }
            if i >= name_start {
                sc += 12;
            }
            if k > 0 && idx[k - 1] + 1 == i {
                sc += 20;
            }
            if t[i] == query.chars().nth(k).unwrap_or(' ') {
                sc += 1;
            }
        }
        sc -= (idx[0] as i32 - name_start as i32).abs().min(30);
        sc -= (t.len() as i32 / 8).min(20);
        if best.as_ref().is_none_or(|(b, _)| sc > *b) {
            best = Some((sc, idx));
        }
    }
    best
}

fn is_word_start(t: &[char], i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let p = t[i - 1];
    let c = t[i];
    matches!(p, ' ' | '_' | '-' | '.' | '\\' | '/' | ':' | '(' | '[') || (p.is_lowercase() && c.is_uppercase()) || (!p.is_ascii_digit() && c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_in_order_only() {
        assert!(score("abc", "a_b_c").is_some());
        assert!(score("cba", "abc").is_none());
        assert!(score("", "anything").is_some());
    }

    #[test]
    fn prefers_word_starts_and_file_names() {
        let a = score("tog", "Toggle comment").unwrap().0;
        let b = score("tog", "Settings: photograph").unwrap().0;
        assert!(a > b);
        let f = score("main", "C:\\src\\main.rs").unwrap().0;
        let d = score("main", "C:\\main\\src\\lib.rs").unwrap().0;
        assert!(f > d);
        let (_, idx) = score("sa", "File: Save As").unwrap();
        assert_eq!(idx, vec![6, 7]);
    }
}
