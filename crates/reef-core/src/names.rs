//! GitHub's naming rules, so anything valid on GitHub is valid on Reef and vice versa.

use crate::Error;

/// A login (user or organization): 1-39 characters, alphanumeric or single hyphens,
/// cannot begin or end with a hyphen.
pub fn validate_login(s: &str) -> Result<(), Error> {
    let ok = !s.is_empty()
        && s.len() <= 39
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--");
    if !ok {
        return Err(Error::field(
            "User",
            "login",
            "invalid",
            "login must be 1-39 letters, digits or single hyphens and cannot start or end with a hyphen",
        ));
    }
    if RESERVED_LOGINS.contains(&s.to_ascii_lowercase().as_str()) {
        return Err(Error::field("User", "login", "reserved", format!("'{s}' is a reserved name and cannot be used as a login")));
    }
    Ok(())
}

/// A repository name: 1-100 characters of `[A-Za-z0-9._-]`, not `.` or `..`.
pub fn validate_repo_name(s: &str) -> Result<(), Error> {
    let ok = !s.is_empty()
        && s.len() <= 100
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && s != "."
        && s != ".."
        && !s.ends_with(".git");
    if ok { Ok(()) } else { Err(Error::field("Repository", "name", "invalid", "name is invalid")) }
}

/// Sanitize a requested repository name the way GitHub does: replace runs of
/// disallowed characters with `-`.
pub fn sanitize_repo_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            last_dash = c == '-';
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_end_matches(".git").to_string()
}

/// A git reference name that we accept as a branch: no control chars, no `..`, no `~^:?*[\`.
pub fn validate_branch(s: &str) -> Result<(), Error> {
    let bad = s.is_empty()
        || s.starts_with('/')
        || s.ends_with('/')
        || s.ends_with('.')
        || s.ends_with(".lock")
        || s.contains("..")
        || s.contains("//")
        || s.contains("@{")
        || s.chars().any(|c| c.is_control() || " ~^:?*[\\".contains(c));
    if bad { Err(Error::validation("invalid branch name")) } else { Ok(()) }
}

/// Logins that would collide with top-level routes.
pub const RESERVED_LOGINS: &[&str] = &[
    "api",
    "login",
    "logout",
    "signup",
    "join",
    "settings",
    "site",
    "root",
    "orgs",
    "organizations",
    "users",
    "repos",
    "repositories",
    "new",
    "search",
    "explore",
    "notifications",
    "marketplace",
    "pulls",
    "issues",
    "static",
    "assets",
    "graphql",
    "sessions",
    "site",
    "about",
    "security",
    "features",
    "topics",
    "trending",
    "sponsors",
    "codespaces",
    "packages",
    "projects",
    "wiki",
    "health",
    "healthz",
    "readyz",
    "metrics",
    "oauth",
    "device",
    "actions",
    "runners",
    "avatars",
    "products",
    "user",
    // The product itself, so the name stays free for integrations.
    "reef",
    "_",
    "-",
    "info",
    "objects",
    "refs",
    "git-upload-pack",
    "git-receive-pack",
];

/// `@login` mentions in free text, skipping email addresses and code spans.
pub fn mentions(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_code = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            in_code = !in_code;
            i += 1;
            continue;
        }
        if !in_code && c == '@' && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-') && j - start < 39 {
                j += 1;
            }
            if j > start && (j >= chars.len() || chars[j] != '@') {
                let login: String = chars[start..j].iter().collect();
                if validate_login(&login).is_ok() {
                    out.push(login);
                }
            }
            i = j.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logins() {
        assert!(validate_login("rohit").is_ok());
        assert!(validate_login("yadv-labs").is_ok());
        assert!(validate_login("-bad").is_err());
        assert!(validate_login("api").is_err());
        assert!(validate_login("a--b").is_err());
    }
    #[test]
    fn repos() {
        assert!(validate_repo_name("reef").is_ok());
        assert!(validate_repo_name("my.repo_v2").is_ok());
        assert!(validate_repo_name("bad name").is_err());
        assert_eq!(sanitize_repo_name("My Cool Repo!"), "My-Cool-Repo-");
    }
}
