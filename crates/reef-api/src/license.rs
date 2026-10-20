//! License detection by matching distinctive phrases of common SPDX licenses.

use serde_json::{Value, json};

/// (spdx id, name, key, phrases that must all appear)
const LICENSES: &[(&str, &str, &str, &[&str])] = &[
    ("Apache-2.0", "Apache License 2.0", "apache-2.0", &["apache license", "version 2.0"]),
    ("AGPL-3.0", "GNU Affero General Public License v3.0", "agpl-3.0", &["gnu affero general public license", "version 3"]),
    ("LGPL-3.0", "GNU Lesser General Public License v3.0", "lgpl-3.0", &["gnu lesser general public license", "version 3"]),
    ("LGPL-2.1", "GNU Lesser General Public License v2.1", "lgpl-2.1", &["gnu lesser general public license", "version 2.1"]),
    ("GPL-3.0", "GNU General Public License v3.0", "gpl-3.0", &["gnu general public license", "version 3"]),
    ("GPL-2.0", "GNU General Public License v2.0", "gpl-2.0", &["gnu general public license", "version 2"]),
    ("MPL-2.0", "Mozilla Public License 2.0", "mpl-2.0", &["mozilla public license", "2.0"]),
    (
        "BSD-3-Clause",
        "BSD 3-Clause \"New\" or \"Revised\" License",
        "bsd-3-clause",
        &["redistribution and use in source and binary forms", "neither the name of"],
    ),
    (
        "BSD-2-Clause",
        "BSD 2-Clause \"Simplified\" License",
        "bsd-2-clause",
        &["redistribution and use in source and binary forms", "this list of conditions"],
    ),
    ("ISC", "ISC License", "isc", &["permission to use, copy, modify, and/or distribute this software for any purpose"]),
    ("Unlicense", "The Unlicense", "unlicense", &["this is free and unencumbered software released into the public domain"]),
    ("CC0-1.0", "Creative Commons Zero v1.0 Universal", "cc0-1.0", &["cc0 1.0 universal"]),
    ("MIT", "MIT License", "mit", &["permission is hereby granted, free of charge", "without restriction"]),
    ("BSL-1.0", "Boost Software License 1.0", "bsl-1.0", &["boost software license"]),
    ("EUPL-1.2", "European Union Public License 1.2", "eupl-1.2", &["european union public licence"]),
];

/// Detect a license from the text of a license file. Unknown text yields `other`.
pub fn detect(text: &str) -> Value {
    let lower: String = text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
    for (spdx, name, key, phrases) in LICENSES {
        if phrases.iter().all(|p| lower.contains(p)) {
            return json!({"key": key, "name": name, "spdx_id": spdx, "url": format!("https://api.github.com/licenses/{key}"), "node_id": format!("L_{key}")});
        }
    }
    json!({"key": "other", "name": "Other", "spdx_id": "NOASSERTION", "url": Value::Null, "node_id": "L_other"})
}

#[cfg(test)]
mod tests {
    #[test]
    fn detects() {
        assert_eq!(
            super::detect("MIT License\n\nPermission is hereby granted, free of charge, to any person ... without restriction")["spdx_id"],
            "MIT"
        );
        assert_eq!(super::detect("Apache License\nVersion 2.0, January 2004")["spdx_id"], "Apache-2.0");
        assert_eq!(super::detect("hello")["key"], "other");
    }
}
