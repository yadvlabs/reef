//! Server-side rendering of user content: GitHub Flavored Markdown, sanitised.

use comrak::{Options, markdown_to_html};

/// Markdown rendered in the context of a repository: `#123` links to issues and `@login`
/// to profiles, outside code.
pub fn markdown_in_repo(src: &str, cfg: &crate::Config, rw: &reef_db::repos::RepoWithOwner) -> String {
    let site = cfg.site_url();
    let full = rw.full_name();
    let linked = autolink(src, &|kind, text| match kind {
        Ref::Issue => format!("[#{text}]({site}/{full}/issues/{text})"),
        Ref::Mention => format!("[@{text}]({site}/{text})"),
    });
    markdown(&linked)
}

#[derive(Clone, Copy)]
enum Ref {
    Issue,
    Mention,
}

/// Replace `#123` and `@login` outside code spans and fenced blocks using `f`.
fn autolink(src: &str, f: &dyn Fn(Ref, &str) -> String) -> String {
    let mut out = String::with_capacity(src.len() + 64);
    let mut in_fence = false;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
            continue;
        }
        if in_fence || trimmed.starts_with("    ") {
            out.push_str(line);
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        let mut in_code = false;
        let mut in_link = 0i32;
        while i < chars.len() {
            let c = chars[i];
            if c == '`' {
                in_code = !in_code;
                out.push(c);
                i += 1;
                continue;
            }
            if !in_code {
                if c == '[' {
                    in_link += 1;
                }
                if c == ']' {
                    in_link = (in_link - 1).max(0);
                }
                let prev_ok =
                    i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '/' || chars[i - 1] == '[' || chars[i - 1] == '(');
                if in_link == 0 && prev_ok && (c == '#' || c == '@') {
                    let start = i + 1;
                    let mut j = start;
                    if c == '#' {
                        while j < chars.len() && chars[j].is_ascii_digit() {
                            j += 1;
                        }
                    } else {
                        while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '-') && j - start < 39 {
                            j += 1;
                        }
                    }
                    let boundary_ok = j >= chars.len() || !(chars[j].is_alphanumeric() || chars[j] == '@' || chars[j] == '.');
                    if j > start && boundary_ok {
                        let text: String = chars[start..j].iter().collect();
                        let valid = if c == '#' { true } else { reef_core::names::validate_login(&text).is_ok() };
                        if valid {
                            out.push_str(&f(if c == '#' { Ref::Issue } else { Ref::Mention }, &text));
                            i = j;
                            continue;
                        }
                    }
                }
            }
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Markdown from a file in a repository (a README, docs): relative links point at the file's
/// directory at the same revision, as on GitHub. Images resolve to the raw file so they render,
/// other links to the file view (a trailing slash means a directory).
pub fn markdown_file(src: &str, rw: &reef_db::repos::RepoWithOwner, rev: &str, path: &str) -> String {
    let full = rw.full_name();
    let dir = match path.rfind('/') {
        Some(i) => &path[..i],
        None => "",
    };
    rewrite_relative_urls(
        &markdown(src),
        &format!("/{full}/blob/{rev}/"),
        &format!("/{full}/tree/{rev}/"),
        &format!("/{full}/raw/{rev}/"),
        dir,
    )
}

/// Rewrite relative `href` and `src` attributes of `<a>`, `<img>` and `<source>` tags in
/// sanitised HTML. `dir` is the directory the links are relative to (empty for the root).
/// Absolute URLs, fragment-only and query-only links are left alone.
fn rewrite_relative_urls(html: &str, blob_base: &str, tree_base: &str, raw_base: &str, dir: &str) -> String {
    let mut out = String::with_capacity(html.len() + 256);
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let tag = &rest[lt..];
        let Some(gt) = tag.find('>') else {
            out.push_str(tag);
            return out;
        };
        let (open, after) = tag.split_at(gt + 1);
        let lower = open.get(..5).map(|s| s.to_ascii_lowercase()).unwrap_or_default();
        let attr = if lower.starts_with("<a ") || lower.starts_with("<a\t") || lower.starts_with("<a\n") {
            Some(("href=\"", false))
        } else if lower.starts_with("<img ") || lower.starts_with("<source ") || lower.starts_with("<sourc") {
            Some(("src=\"", true))
        } else {
            None
        };
        match attr.and_then(|(name, image)| find_attr(open, name).map(|(a, b)| (a, b, image))) {
            Some((start, end, image)) => {
                let value = &open[start..end];
                out.push_str(&open[..start]);
                match resolve_relative(value, dir) {
                    Some(resolved) => {
                        let base = if image {
                            raw_base
                        } else if resolved.path.ends_with('/') || resolved.path.is_empty() {
                            tree_base
                        } else {
                            blob_base
                        };
                        out.push_str(base);
                        out.push_str(resolved.path.trim_end_matches('/'));
                        out.push_str(resolved.suffix);
                    }
                    None => out.push_str(value),
                }
                out.push_str(&open[end..]);
            }
            None => out.push_str(open),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The value range of a `name="..."` attribute inside one tag (the serialiser always
/// double-quotes), as byte offsets into `open`.
fn find_attr(open: &str, name: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(i) = open[from..].find(name) {
        let at = from + i;
        let before = open[..at].chars().next_back();
        if matches!(before, Some(c) if c.is_whitespace()) {
            let start = at + name.len();
            let end = start + open[start..].find('"')?;
            return Some((start, end));
        }
        from = at + name.len();
    }
    None
}

struct Resolved<'a> {
    /// The repository path, `..` and `.` folded, with a trailing slash kept when given.
    path: String,
    /// `?query` and `#fragment`, untouched.
    suffix: &'a str,
}

/// `Some` for a link that is relative to the repository, `None` for anything else: absolute
/// URLs (`https:`, `mailto:`, `data:`, protocol-relative), site-absolute paths, and links that
/// are only a fragment or a query.
fn resolve_relative<'a>(url: &'a str, dir: &str) -> Option<Resolved<'a>> {
    if url.is_empty() || url.starts_with('/') || url.starts_with('#') || url.starts_with('?') {
        return None;
    }
    let scheme_end = url.find(|c: char| !(c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.'));
    if let Some(i) = scheme_end
        && i > 0
        && url[i..].starts_with(':')
    {
        return None;
    }
    let cut = url.find(['?', '#']).unwrap_or(url.len());
    let (path, suffix) = url.split_at(cut);
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut joined = parts.join("/");
    if path.ends_with('/') && !joined.is_empty() {
        joined.push('/');
    }
    Some(Resolved { path: joined, suffix })
}

pub fn markdown(src: &str) -> String {
    let mut o = Options::default();
    o.extension.strikethrough = true;
    o.extension.table = true;
    o.extension.autolink = true;
    o.extension.tasklist = true;
    o.extension.footnotes = true;
    o.extension.header_id_prefix = Some("user-content-".into());
    o.extension.alerts = true;
    o.extension.math_dollars = true;
    o.extension.shortcodes = true;
    o.render.github_pre_lang = true;
    o.render.gfm_quirks = true;
    o.render.tasklist_classes = true;
    // Unsafe HTML is allowed through comrak and then sanitised by ammonia, which keeps the
    // useful subset (images, details, tables) and drops scripts and handlers.
    o.render.r#unsafe = true;
    let html = markdown_to_html(src, &o);
    let mut b = ammonia::Builder::default();
    b.add_tags(["details", "summary", "input", "kbd", "sup", "sub", "del", "ins", "picture", "source"]);
    b.add_tag_attributes("input", ["type", "checked", "disabled"]);
    b.add_tag_attributes("source", ["srcset", "media", "type"]);
    b.add_tag_attributes("th", ["align"]);
    b.add_tag_attributes("td", ["align"]);
    b.add_generic_attributes(["id", "class", "align"]);
    b.link_rel(Some("nofollow noopener"));
    b.clean(&html).to_string()
}

#[cfg(test)]
mod tests {
    #[test]
    fn autolinks_outside_code() {
        let f = |k: super::Ref, t: &str| match k {
            super::Ref::Issue => format!("<I{t}>"),
            super::Ref::Mention => format!("<M{t}>"),
        };
        assert_eq!(super::autolink("fix #12 by @rohit", &f), "fix <I12> by <Mrohit>");
        assert_eq!(super::autolink("see `#12` and mail@x.com", &f), "see `#12` and mail@x.com");
        assert_eq!(super::autolink("```\n#1 @a\n```\n#2", &f), "```\n#1 @a\n```\n<I2>");
        assert_eq!(super::autolink("[link #3](http://x/#4)", &f), "[link #3](http://x/#4)");
    }

    #[test]
    fn relative_links_point_into_the_repository() {
        let html = super::rewrite_relative_urls(
            "<p><img src=\"build/logo.svg\" alt=\"x\"> <a href=\"docs/start.md#run\">a</a> <a href=\"../LICENSE\">b</a> <a href=\"deploy/\">c</a> <a href=\"https://x.test/y\">d</a> <a href=\"#top\">e</a> <a href=\"mailto:a@b\">f</a> <a href=\"/abs\">g</a> <a href=\"./SETUP.md?x=1\">h</a></p>",
            "/o/r/blob/main/",
            "/o/r/tree/main/",
            "/o/r/raw/main/",
            "docs",
        );
        assert!(html.contains("src=\"/o/r/raw/main/docs/build/logo.svg\""), "{html}");
        assert!(html.contains("href=\"/o/r/blob/main/docs/docs/start.md#run\""), "{html}");
        assert!(html.contains("href=\"/o/r/blob/main/LICENSE\""), "{html}");
        assert!(html.contains("href=\"/o/r/tree/main/docs/deploy\""), "{html}");
        assert!(html.contains("href=\"https://x.test/y\""), "{html}");
        assert!(html.contains("href=\"#top\"") && html.contains("href=\"mailto:a@b\"") && html.contains("href=\"/abs\""), "{html}");
        assert!(html.contains("href=\"/o/r/blob/main/docs/SETUP.md?x=1\""), "{html}");
        // The root README: no directory prefix, and HTML in the markdown is covered too.
        let root = super::rewrite_relative_urls(
            super::markdown("<img src=\"branding/mark.svg\" align=\"right\" width=\"96\">\n\n[setup](SETUP.md)").as_str(),
            "/o/r/blob/main/",
            "/o/r/tree/main/",
            "/o/r/raw/main/",
            "",
        );
        assert!(root.contains("src=\"/o/r/raw/main/branding/mark.svg\""), "{root}");
        assert!(root.contains("align=\"right\"") && root.contains("width=\"96\""), "{root}");
        assert!(root.contains("href=\"/o/r/blob/main/SETUP.md\""), "{root}");
    }

    #[test]
    fn renders_gfm_and_strips_scripts() {
        let html = super::markdown("# Hi\n\n- [x] done\n\n<script>alert(1)</script>\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(html.contains("<h1"));
        assert!(html.contains("<table"));
        assert!(html.contains("type=\"checkbox\""));
        assert!(!html.contains("<script"));
    }
}
