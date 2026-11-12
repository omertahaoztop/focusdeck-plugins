use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

const PROTOCOL: u32 = 1;
const SHA256_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub description: String,
    pub version: String,
    pub targets: BTreeMap<String, Target>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    #[serde(default)]
    plugin: Vec<Stored>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    name: String,
    description: String,
    version: String,
    protocol: u32,
    targets: BTreeMap<String, Target>,
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn validate(entry: &Entry) -> Result<(), String> {
    if !is_identifier(&entry.name) {
        return Err(format!("Geçersiz plugin adı: {}", entry.name));
    }
    if entry.version.is_empty() {
        return Err("Sürüm boş".into());
    }
    if entry.targets.is_empty() {
        return Err("Hedef listesi boş".into());
    }
    for (triple, target) in &entry.targets {
        if !is_identifier(triple) {
            return Err(format!("Geçersiz hedef adı: {triple}"));
        }
        if !target.url.starts_with("https://") || target.url.chars().any(char::is_whitespace) {
            return Err(format!("{triple} için adres https:// ile başlamalı"));
        }
        let hex = target.sha256.len() == SHA256_LEN
            && target
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !hex {
            return Err(format!(
                "{triple} için sha256 64 küçük onaltılık hane olmalı"
            ));
        }
    }
    Ok(())
}

pub fn set_entry(index: &str, entry: Entry) -> Result<String, String> {
    validate(&entry)?;
    let mut parsed: Index =
        toml::from_str(index).map_err(|e| format!("Dizin okunamadı: {}", e.message()))?;
    parsed.plugin.retain(|p| p.name != entry.name);
    parsed.plugin.push(Stored {
        name: entry.name,
        description: entry.description,
        version: entry.version,
        protocol: PROTOCOL,
        targets: entry.targets,
    });
    parsed.plugin.sort_by(|a, b| a.name.cmp(&b.name));
    toml::to_string(&parsed).map_err(|e| format!("Dizin yazılamadı: {e}"))
}

pub fn check_tag(tag: &str, name: &str, version: &str) -> Result<(), String> {
    let expected = format!("{name}-v{version}");
    if tag == expected {
        Ok(())
    } else {
        Err(format!("Etiket {tag}, beklenen {expected}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeMap;

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const X86: &str = "x86_64-unknown-linux-gnu";
    const ARM: &str = "aarch64-unknown-linux-gnu";

    #[derive(Deserialize)]
    struct Index {
        plugin: Vec<IndexEntry>,
    }

    #[derive(Deserialize)]
    struct IndexEntry {
        name: String,
        description: String,
        version: String,
        protocol: u32,
        targets: BTreeMap<String, IndexTarget>,
    }

    #[derive(Deserialize)]
    struct IndexTarget {
        url: String,
        sha256: String,
    }

    fn entry(name: &str, version: &str) -> Entry {
        Entry {
            name: name.into(),
            description: format!("{name} açıklaması"),
            version: version.into(),
            targets: BTreeMap::from([
                (
                    X86.to_string(),
                    Target {
                        url: format!("https://example.com/{name}-{X86}"),
                        sha256: SHA_A.into(),
                    },
                ),
                (
                    ARM.to_string(),
                    Target {
                        url: format!("https://example.com/{name}-{ARM}"),
                        sha256: SHA_B.into(),
                    },
                ),
            ]),
        }
    }

    fn parse(text: &str) -> Index {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn set_into_empty_index() {
        let out = set_entry("", entry("redmine", "0.1.0")).unwrap();
        let index = parse(&out);
        assert_eq!(index.plugin.len(), 1);
        assert_eq!(index.plugin[0].name, "redmine");
    }

    #[test]
    fn set_replaces_same_name_keeps_others_sorted() {
        let first = set_entry("", entry("zulip", "0.1.0")).unwrap();
        let second = set_entry(&first, entry("redmine", "0.1.0")).unwrap();
        let third = set_entry(&second, entry("gmail", "0.1.0")).unwrap();
        let out = set_entry(&third, entry("redmine", "0.2.0")).unwrap();
        let index = parse(&out);
        let seen: Vec<_> = index
            .plugin
            .iter()
            .map(|p| (p.name.as_str(), p.version.as_str()))
            .collect();
        assert_eq!(
            seen,
            [("gmail", "0.1.0"), ("redmine", "0.2.0"), ("zulip", "0.1.0")]
        );
    }

    #[test]
    fn output_parses_as_index() {
        let out = set_entry("", entry("redmine", "0.1.0")).unwrap();
        let plugin = &parse(&out).plugin[0];
        assert_eq!(plugin.description, "redmine açıklaması");
        assert_eq!(plugin.protocol, 1);
        assert_eq!(plugin.targets.len(), 2);
        let x86 = &plugin.targets[X86];
        assert_eq!(x86.url, format!("https://example.com/redmine-{X86}"));
        assert_eq!(x86.sha256, SHA_A);
        assert_eq!(plugin.targets[ARM].sha256, SHA_B);
        assert!(out.contains("[[plugin]]"));
        assert!(out.contains(&format!("[plugin.targets.{X86}]")));
    }

    #[test]
    fn rejects_bad_sha256() {
        for bad in [
            "abc",
            &SHA_A.to_uppercase(),
            &format!("{}g", &SHA_A[..63]),
            &format!("{SHA_A}a"),
        ] {
            let mut e = entry("redmine", "0.1.0");
            e.targets.get_mut(X86).unwrap().sha256 = bad.into();
            assert!(set_entry("", e).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_non_https_url() {
        for bad in [
            "http://example.com/x",
            "file:///tmp/x",
            "example.com/x",
            "https://example.com/a b",
        ] {
            let mut e = entry("redmine", "0.1.0");
            e.targets.get_mut(ARM).unwrap().url = bad.into();
            assert!(set_entry("", e).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_empty_targets() {
        let mut e = entry("redmine", "0.1.0");
        e.targets.clear();
        assert!(set_entry("", e).is_err());
    }

    #[test]
    fn rejects_unsafe_names() {
        for bad in ["", "a b", "a\"b", "a.b"] {
            assert!(set_entry("", entry(bad, "0.1.0")).is_err(), "{bad}");
        }
        let mut e = entry("redmine", "0.1.0");
        let target = e.targets.remove(ARM).unwrap();
        e.targets.insert("arm 64".into(), target);
        assert!(set_entry("", e).is_err());
    }

    #[test]
    fn rejects_malformed_existing_index() {
        assert!(set_entry("[[plugin]\nname", entry("redmine", "0.1.0")).is_err());
    }

    #[test]
    fn tag_must_match_package_version() {
        assert!(check_tag("redmine-v0.1.0", "redmine", "0.1.0").is_ok());
        assert!(check_tag("redmine-v0.2.0", "redmine", "0.1.0").is_err());
        assert!(check_tag("zulip-v0.1.0", "redmine", "0.1.0").is_err());
    }
}
