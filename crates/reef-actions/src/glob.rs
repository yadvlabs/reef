//! GitHub's filter-pattern glob for `branches`, `tags` and `paths`: `*` matches anything but
//! `/`, `**` matches across `/`, `?` one char, `+(a|b)`-style alternatives are not supported
//! (GitHub doesn't either), `[abc]`/`[a-z]` classes, `\` escapes, and a leading `!` negates.

pub fn matches(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    m(&p, &t)
}

fn m(p: &[char], t: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' if p.len() > 1 && p[1] == '*' => {
            // `**` matches any sequence including separators; also swallow a following `/`.
            let rest = if p.len() > 2 && p[2] == '/' { &p[3..] } else { &p[2..] };
            (0..=t.len()).any(|i| m(rest, &t[i..]))
        }
        '*' => (0..=t.len()).take_while(|&i| i == 0 || t[i - 1] != '/').any(|i| m(&p[1..], &t[i..])),
        '?' => !t.is_empty() && t[0] != '/' && m(&p[1..], &t[1..]),
        '[' => {
            let Some(end) = p.iter().position(|&c| c == ']') else { return !t.is_empty() && t[0] == '[' && m(&p[1..], &t[1..]) };
            if t.is_empty() {
                return false;
            }
            let class = &p[1..end];
            let (negate, class) =
                if class.first() == Some(&'!') || class.first() == Some(&'^') { (true, &class[1..]) } else { (false, class) };
            let mut hit = false;
            let mut i = 0;
            while i < class.len() {
                if i + 2 < class.len() && class[i + 1] == '-' {
                    if class[i] <= t[0] && t[0] <= class[i + 2] {
                        hit = true;
                    }
                    i += 3;
                } else {
                    if class[i] == t[0] {
                        hit = true;
                    }
                    i += 1;
                }
            }
            hit != negate && m(&p[end + 1..], &t[1..])
        }
        '\\' if p.len() > 1 => !t.is_empty() && t[0] == p[1] && m(&p[2..], &t[1..]),
        c => !t.is_empty() && t[0] == c && m(&p[1..], &t[1..]),
    }
}

/// Apply a list of patterns in order; `!` patterns negate. A list with only negations
/// includes everything except the negated names, like GitHub.
pub fn filter(patterns: &[String], text: &str) -> bool {
    if patterns.is_empty() {
        return true;
    }
    let mut result = patterns.iter().all(|p| p.starts_with('!'));
    for p in patterns {
        if let Some(neg) = p.strip_prefix('!') {
            if matches(neg, text) {
                result = false;
            }
        } else if matches(p, text) {
            result = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(matches("main", "main"));
        assert!(matches("release/*", "release/1.0"));
        assert!(!matches("release/*", "release/1.0/hotfix"));
        assert!(matches("release/**", "release/1.0/hotfix"));
        assert!(matches("**", "a/b/c"));
        assert!(matches("*.md", "README.md"));
        assert!(!matches("*.md", "docs/README.md"));
        assert!(matches("**/*.md", "docs/README.md"));
        assert!(matches("**/*.md", "README.md"));
        assert!(matches("v[0-9].*", "v1.2"));
        assert!(matches("feature/??", "feature/ab"));
        assert!(matches("src/**", "src/main.rs"));
    }

    #[test]
    fn filters() {
        let pats = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(filter(&pats(&["main", "release/**"]), "release/2"));
        assert!(!filter(&pats(&["main"]), "dev"));
        assert!(filter(&pats(&["!docs/**"]), "src/a.rs"));
        assert!(!filter(&pats(&["!docs/**"]), "docs/a.md"));
        assert!(!filter(&pats(&["**", "!docs/**"]), "docs/a.md"));
        assert!(filter(&pats(&["**", "!docs/**", "docs/keep.md"]), "docs/keep.md"));
    }
}
