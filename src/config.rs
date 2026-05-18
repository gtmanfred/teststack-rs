use anyhow::{Context, Result};
use indexmap::IndexMap;
use semver::Version;
use std::path::Path;
use toml::Value;

#[derive(Debug, Clone)]
pub struct Config {
    pub root: Value,
}

impl Default for Config {
    fn default() -> Self {
        Self { root: Value::Table(Default::default()) }
    }
}

impl Config {
    pub fn load(main: &Path, local: &Path) -> Result<Self> {
        let mut root = if main.exists() {
            let txt = std::fs::read_to_string(main).with_context(|| format!("read {main:?}"))?;
            toml::from_str::<Value>(&txt).with_context(|| format!("parse {main:?}"))?
        } else {
            Value::Table(Default::default())
        };
        if local.exists() {
            let txt = std::fs::read_to_string(local).with_context(|| format!("read {local:?}"))?;
            let over: Value = toml::from_str(&txt).with_context(|| format!("parse {local:?}"))?;
            merge(&mut root, over);
        }
        Ok(Self { root })
    }

    pub fn check_min_version(&self) -> Result<()> {
        let Some(raw) = self.get_string("tests.min_version") else {
            return Ok(());
        };
        let min = Version::parse(raw.trim_start_matches('v'))
            .with_context(|| format!("invalid tests.min_version: {raw}"))?;
        let cur = Version::parse(env!("CARGO_PKG_VERSION"))?;
        if min > cur {
            anyhow::bail!("teststack {cur} too old; need >= {min}");
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let mut cur = &self.root;
        for part in key.split('.') {
            cur = cur.as_table()?.get(part)?;
        }
        Some(interpolate(cur.clone()))
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        self.get(key).and_then(|v| v.as_str().map(|s| s.to_string()))
    }

    pub fn get_table(&self, key: &str) -> IndexMap<String, Value> {
        match self.get(key) {
            Some(Value::Table(t)) => t.into_iter().collect(),
            _ => Default::default(),
        }
    }
}

fn merge(into: &mut Value, from: Value) {
    match (into, from) {
        (Value::Table(a), Value::Table(b)) => {
            for (k, v) in b {
                match a.get_mut(&k) {
                    Some(slot) if slot.is_table() && v.is_table() => merge(slot, v),
                    _ => {
                        a.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

fn interpolate(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(interp_str(&s)),
        Value::Table(t) => Value::Table(t.into_iter().map(|(k, v)| (k, interpolate(v))).collect()),
        Value::Array(a) => Value::Array(a.into_iter().map(interpolate).collect()),
        other => other,
    }
}

pub fn interp_str(s: &str) -> String {
    interp_with(s, |k| std::env::var(k).ok())
}

pub fn interp_with<F: Fn(&str) -> Option<String>>(s: &str, lookup: F) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            if let Some(end) = s[i + 1..].find('}') {
                let key = &s[i + 1..i + 1 + end];
                if !key.contains('{') {
                    if let Some(val) = lookup(key) {
                        out.push_str(&val);
                        i = i + 1 + end + 1;
                        continue;
                    } else {
                        out.push_str(&s[i..i + 1 + end + 1]);
                        i = i + 1 + end + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(s: &str) -> Config {
        Config {
            root: toml::from_str(s).unwrap(),
        }
    }

    #[test]
    fn dotted_lookup() {
        let c = cfg("[tests]\nstage='alpha'\n");
        assert_eq!(c.get_string("tests.stage").as_deref(), Some("alpha"));
        assert!(c.get("missing.thing").is_none());
    }

    #[test]
    fn merge_local_overrides() {
        let mut a: Value = toml::from_str("[x]\na=1\nb=2\n").unwrap();
        let b: Value = toml::from_str("[x]\nb=3\nc=4\n").unwrap();
        merge(&mut a, b);
        let t = a.get("x").unwrap().as_table().unwrap();
        assert_eq!(t.get("a").unwrap().as_integer(), Some(1));
        assert_eq!(t.get("b").unwrap().as_integer(), Some(3));
        assert_eq!(t.get("c").unwrap().as_integer(), Some(4));
    }

    #[test]
    fn env_interpolation_missing_key_passthrough() {
        let r = interp_with("hello {NOPE} world", |_| None);
        assert_eq!(r, "hello {NOPE} world");
        let r = interp_with("hello {NAME}", |k| (k == "NAME").then(|| "bob".into()));
        assert_eq!(r, "hello bob");
    }

    #[test]
    fn semicolon_keys_passthrough() {
        let r = interp_with("{PORT;5432/tcp}", |_| None);
        assert_eq!(r, "{PORT;5432/tcp}");
        let r = interp_with("{PORT;5432/tcp}", |k| (k == "PORT;5432/tcp").then(|| "5432".into()));
        assert_eq!(r, "5432");
    }
}
