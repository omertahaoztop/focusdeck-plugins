use std::collections::BTreeMap;

use semver::Version;
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

fn validate(entry: &Entry) -> Result<Version, String> {
    if !is_identifier(&entry.name) {
        return Err(format!("Invalid plugin name: {}", entry.name));
    }
    let version = Version::parse(&entry.version)
        .map_err(|_| format!("Version is not semver: {}", entry.version))?;
    if entry.targets.is_empty() {
        return Err("Target list is empty".into());
    }
    for (triple, target) in &entry.targets {
        if !is_identifier(triple) {
            return Err(format!("Invalid target name: {triple}"));
        }
        if !target.url.starts_with("https://") || target.url.chars().any(char::is_whitespace) {
            return Err(format!("URL for {triple} must start with https://"));
        }
        let hex = target.sha256.len() == SHA256_LEN
            && target
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !hex {
            return Err(format!(
                "sha256 for {triple} must be 64 lowercase hex digits"
            ));
        }
    }
    Ok(version)
}

pub fn set_entry(index: &str, entry: Entry) -> Result<String, String> {
    let version = validate(&entry)?;
    let mut parsed: Index =
        toml::from_str(index).map_err(|e| format!("Could not read the index: {}", e.message()))?;
    if let Some(published) = parsed.plugin.iter().find(|p| p.name == entry.name) {
        let current = Version::parse(&published.version).map_err(|_| {
            format!(
                "Published version of {} is not semver: {}",
                entry.name, published.version
            )
        })?;
        // a re-run of the same tag is fine, an older tag must not replace what clients already trust
        if version.cmp_precedence(&current).is_lt() {
            return Err(format!(
                "{} {} is older than the published {}",
                entry.name, entry.version, published.version
            ));
        }
    }
    parsed.plugin.retain(|p| p.name != entry.name);
    parsed.plugin.push(Stored {
        name: entry.name,
        description: entry.description,
        version: entry.version,
        protocol: PROTOCOL,
        targets: entry.targets,
    });
    parsed.plugin.sort_by(|a, b| a.name.cmp(&b.name));
    toml::to_string(&parsed).map_err(|e| format!("Could not write the index: {e}"))
}

pub fn check_tag(tag: &str, name: &str, version: &str) -> Result<(), String> {
    let expected = format!("{name}-v{version}");
    if tag == expected {
        Ok(())
    } else {
        Err(format!("Tag is {tag}, expected {expected}"))
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
            description: format!("{name} description"),
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
        assert_eq!(plugin.description, "redmine description");
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
    fn rejects_versions_that_are_not_semver() {
        for bad in ["", "1", "1.0", "v1.0.0", "1.0.0 ", "latest", "01.0.0"] {
            let err = set_entry("", entry("redmine", bad)).unwrap_err();
            assert_eq!(err, format!("Version is not semver: {bad}"));
        }
        for good in ["0.1.0", "1.2.3-rc.1", "1.2.3+build.5"] {
            assert!(set_entry("", entry("redmine", good)).is_ok(), "{good}");
        }
    }

    #[test]
    fn refuses_to_move_a_plugin_backwards() {
        let published = set_entry("", entry("redmine", "0.2.0")).unwrap();
        for older in ["0.1.9", "0.2.0-rc.1", "0.0.1"] {
            let err = set_entry(&published, entry("redmine", older)).unwrap_err();
            assert_eq!(
                err,
                format!("redmine {older} is older than the published 0.2.0")
            );
        }
        let rerun = set_entry(&published, entry("redmine", "0.2.0")).unwrap();
        assert_eq!(rerun, published);
        let newer = set_entry(&published, entry("redmine", "0.10.0")).unwrap();
        assert_eq!(parse(&newer).plugin[0].version, "0.10.0");
        assert!(set_entry(&published, entry("zulip", "0.1.0")).is_ok());
    }

    #[test]
    fn build_metadata_does_not_make_a_version_older() {
        let published = set_entry("", entry("redmine", "1.2.3+build.5")).unwrap();
        assert!(set_entry(&published, entry("redmine", "1.2.3")).is_ok());
    }

    #[test]
    fn refuses_an_index_whose_published_version_is_unreadable() {
        let published = set_entry("", entry("redmine", "0.2.0"))
            .unwrap()
            .replace("0.2.0", "newest");
        let err = set_entry(&published, entry("redmine", "0.3.0")).unwrap_err();
        assert_eq!(err, "Published version of redmine is not semver: newest");
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
