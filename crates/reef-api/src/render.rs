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
    fn renders_gfm_and_strips_scripts() {
        let html = super::markdown("# Hi\n\n- [x] done\n\n<script>alert(1)</script>\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(html.contains("<h1"));
        assert!(html.contains("<table"));
        assert!(html.contains("type=\"checkbox\""));
        assert!(!html.contains("<script"));
    }
}
