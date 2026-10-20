//! `strategy.matrix` expansion with GitHub's `include`/`exclude` semantics.

use crate::{Error, Result};
use indexmap::IndexMap;
use serde_json::Value;

pub type Combination = IndexMap<String, Value>;

/// Expand a (fully evaluated) matrix object into its combinations, in GitHub's order.
pub fn expand(matrix: &Value) -> Result<Vec<Combination>> {
    let Value::Object(m) = matrix else {
        return if matrix.is_null() { Ok(vec![]) } else { Err(Error::Invalid("strategy.matrix must be a mapping".into())) };
    };
    let mut keys: Vec<(&String, &Vec<Value>)> = Vec::new();
    for (k, v) in m {
        if k == "include" || k == "exclude" {
            continue;
        }
        match v {
            Value::Array(items) => keys.push((k, items)),
            _ => return Err(Error::Invalid(format!("matrix.{k} must be a list"))),
        }
    }
    let mut combos: Vec<Combination> = if keys.is_empty() {
        vec![]
    } else {
        let mut acc: Vec<Combination> = vec![Combination::new()];
        for (k, items) in &keys {
            let mut next = Vec::new();
            for c in &acc {
                for item in items.iter() {
                    let mut c2 = c.clone();
                    c2.insert((*k).clone(), item.clone());
                    next.push(c2);
                }
            }
            acc = next;
        }
        acc
    };
    if let Some(Value::Array(excludes)) = m.get("exclude") {
        for ex in excludes {
            let Value::Object(ex) = ex else { continue };
            combos.retain(|c| !ex.iter().all(|(k, v)| c.get(k).map(|cv| cv == v).unwrap_or(false)));
        }
    }
    if let Some(Value::Array(includes)) = m.get("include") {
        let original_keys: Vec<&String> = keys.iter().map(|(k, _)| *k).collect();
        let mut added: Vec<Combination> = Vec::new();
        for inc in includes {
            let Value::Object(inc) = inc else { continue };
            let mut matched = false;
            // Only original combinations can be extended; entries added by earlier includes
            // are never overwritten (GitHub's rule).
            for c in combos.iter_mut() {
                let compatible = inc.iter().all(|(k, v)| match c.get(k) {
                    Some(cv) if original_keys.contains(&k) => cv == v,
                    Some(_) => false, // set by a previous include: would overwrite
                    None => true,
                });
                let touches_original = inc.keys().any(|k| original_keys.contains(&k));
                if compatible && (touches_original || original_keys.is_empty() || inc.keys().all(|k| !original_keys.contains(&k))) {
                    for (k, v) in inc {
                        c.entry(k.clone()).or_insert(v.clone());
                    }
                    matched = true;
                }
            }
            if !matched {
                added.push(inc.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
            }
        }
        combos.extend(added);
    }
    Ok(combos)
}

/// The human name GitHub gives a matrix leg: values joined with ", ".
pub fn leg_name(c: &Combination) -> String {
    c.values().map(crate::expr::to_string).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn product_and_exclude() {
        let m = json!({"os": ["linux", "mac"], "node": [18, 20], "exclude": [{"os": "mac", "node": 18}]});
        let c = expand(&m).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(leg_name(&c[0]), "linux, 18");
        assert_eq!(leg_name(&c[2]), "mac, 20");
    }

    #[test]
    fn include_extends_and_adds() {
        let m = json!({"os": ["linux", "mac"], "include": [{"os": "linux", "experimental": true}, {"os": "windows"}, {"color": "green"}]});
        let c = expand(&m).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].get("experimental"), Some(&json!(true)));
        assert_eq!(c[0].get("color"), Some(&json!("green")));
        assert_eq!(c[1].get("experimental"), None);
        assert_eq!(c[2].get("os"), Some(&json!("windows")));
    }

    #[test]
    fn include_only() {
        let m = json!({"include": [{"a": 1}, {"a": 2}]});
        assert_eq!(expand(&m).unwrap().len(), 2);
    }
}
