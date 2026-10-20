//! A small linguist: language and GitHub color by file name.

/// (extension or exact file name, language, color)
const TABLE: &[(&str, &str, &str)] = &[
    (".rs", "Rust", "#dea584"),
    (".go", "Go", "#00ADD8"),
    (".ts", "TypeScript", "#3178c6"),
    (".tsx", "TypeScript", "#3178c6"),
    (".js", "JavaScript", "#f1e05a"),
    (".jsx", "JavaScript", "#f1e05a"),
    (".mjs", "JavaScript", "#f1e05a"),
    (".cjs", "JavaScript", "#f1e05a"),
    (".py", "Python", "#3572A5"),
    (".rb", "Ruby", "#701516"),
    (".java", "Java", "#b07219"),
    (".kt", "Kotlin", "#A97BFF"),
    (".kts", "Kotlin", "#A97BFF"),
    (".swift", "Swift", "#F05138"),
    (".c", "C", "#555555"),
    (".h", "C", "#555555"),
    (".cc", "C++", "#f34b7d"),
    (".cpp", "C++", "#f34b7d"),
    (".cxx", "C++", "#f34b7d"),
    (".hpp", "C++", "#f34b7d"),
    (".cs", "C#", "#178600"),
    (".php", "PHP", "#4F5D95"),
    (".scala", "Scala", "#c22d40"),
    (".sh", "Shell", "#89e051"),
    (".bash", "Shell", "#89e051"),
    (".zsh", "Shell", "#89e051"),
    (".ps1", "PowerShell", "#012456"),
    (".lua", "Lua", "#000080"),
    (".dart", "Dart", "#00B4AB"),
    (".ex", "Elixir", "#6e4a7e"),
    (".exs", "Elixir", "#6e4a7e"),
    (".erl", "Erlang", "#B83998"),
    (".hs", "Haskell", "#5e5086"),
    (".clj", "Clojure", "#db5855"),
    (".ml", "OCaml", "#ef7a08"),
    (".zig", "Zig", "#ec915c"),
    (".nim", "Nim", "#ffc200"),
    (".r", "R", "#198CE7"),
    (".jl", "Julia", "#a270ba"),
    (".m", "Objective-C", "#438eff"),
    (".vue", "Vue", "#41b883"),
    (".svelte", "Svelte", "#ff3e00"),
    (".html", "HTML", "#e34c26"),
    (".htm", "HTML", "#e34c26"),
    (".css", "CSS", "#663399"),
    (".scss", "SCSS", "#c6538c"),
    (".less", "Less", "#1d365d"),
    (".sql", "PLpgSQL", "#336790"),
    (".proto", "Protocol Buffer", "#e10098"),
    (".graphql", "GraphQL", "#e10098"),
    (".tf", "HCL", "#844FBA"),
    (".hcl", "HCL", "#844FBA"),
    (".nix", "Nix", "#7e7eff"),
    (".dockerfile", "Dockerfile", "#384d54"),
    ("dockerfile", "Dockerfile", "#384d54"),
    ("makefile", "Makefile", "#427819"),
    (".mk", "Makefile", "#427819"),
    (".cmake", "CMake", "#DA3434"),
    (".yml", "YAML", "#cb171e"),
    (".yaml", "YAML", "#cb171e"),
    (".toml", "TOML", "#9c4221"),
    (".json", "JSON", "#292929"),
    (".xml", "XML", "#0060ac"),
    (".md", "Markdown", "#083fa1"),
    (".tex", "TeX", "#3D6117"),
    (".vim", "Vim Script", "#199f4b"),
    (".el", "Emacs Lisp", "#c065db"),
    (".asm", "Assembly", "#6E4C13"),
    (".s", "Assembly", "#6E4C13"),
    (".wasm", "WebAssembly", "#04133b"),
    (".sol", "Solidity", "#AA6746"),
    (".pl", "Perl", "#0298c3"),
    (".groovy", "Groovy", "#4298b8"),
    (".gradle", "Gradle", "#02303a"),
    (".ino", "C++", "#f34b7d"),
];

/// Languages GitHub counts toward the bar (data/markup/config are excluded like linguist).
const DATA_ONLY: &[&str] = &["YAML", "TOML", "JSON", "XML", "Markdown", "TeX"];

pub fn detect(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_lowercase();
    for (ext, lang, _) in TABLE {
        let hit = if ext.starts_with('.') { lower.ends_with(ext) && lower.len() > ext.len() } else { lower == *ext };
        if hit && !DATA_ONLY.contains(lang) {
            return Some(lang);
        }
    }
    None
}

pub fn color(lang: &str) -> &'static str {
    TABLE.iter().find(|(_, l, _)| *l == lang).map(|(_, _, c)| *c).unwrap_or("#ededed")
}
