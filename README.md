# focusdeck-plugins

**English** · [Türkçe](README.tr.md)

Plugins for [focusdeck](https://github.com/omertahaoztop/focusdeck). Each plugin is a separate process that talks to the core with one JSON message per line.

Plugins run with your user's permissions; there is no sandbox. Only install from indexes you trust.

## Install

```
focusdeck plugin install <name>
```

The core picks this machine's target from the `index.toml` index, downloads the binary and checks its sha256. Plugin names: `vikunja`, `redmine`, `zulip`, `gmail`. A plugin's settings go in the `config` table of its box in the profile `~/.config/focusdeck/profiles/<name>.toml` (`config.toml` only holds `default_profile`, `registries` and `capture_command`). Settings whose name ends in `_file` point at the file that holds a secret; its mode must be `0600`.

```toml
[[box]]
id = "redmine"
title = "Redmine plan"
plugin = "redmine"
row = 0
col = 0
config = { url = "https://redmine.example.com", key_file = "~/.config/focusdeck/secrets/redmine" }
```

The keys in the tables below go in this `config` table.

## vikunja

Lists the open tasks of a Vikunja project; marks them done, deletes them, moves them between buckets or reorders them, and creates tasks from dropped notes.

Profile settings:

| Key | Meaning |
|---|---|
| `url` | Vikunja address, for example `https://vikunja.example.com` (`/api/v1` is appended) |
| `token_file` | File holding the API token, mode must be `0600` |
| `project` | Project title |
| `bucket` | Optional bucket title; if given, only that bucket is listed |

In incoming notes, `from` becomes a `kimden:<name>` label, `labels` become labels and `links` become a links paragraph in the description. For an item coming from another box, its source is written to the description as a `focusdeck-source: <plugin>:<id>` line.

## redmine

Lists the open Redmine issues assigned to me. No actions.

Profile settings:

| Key | Meaning |
|---|---|
| `url` | Redmine address, for example `https://redmine.example.com` |
| `key_file` | File holding the API key, mode must be `0600` |

## zulip

Lists starred messages; the `unstar` action removes the star.

Profile settings:

| Key | Meaning |
|---|---|
| `url` | Zulip address, for example `https://zulip.example.com` |
| `email` | Email address of the bot or user |
| `key_file` | File holding the API key, mode must be `0600` |

## gmail

Lists starred Gmail messages with read-only access. No actions.

Profile settings:

| Key | Meaning |
|---|---|
| `client_file` | OAuth client JSON file downloaded from Google Cloud |
| `token_file` | File the refresh token is written to and read from, mode must be `0600` |
| `api_url` | Optional, overrides the Gmail API address; defaults to Google's |

To create the token once:

```
~/.local/share/focusdeck/plugins/gmail login --client-file <path> --token-file <path> [--hint <email>]
```

`focusdeck plugin install gmail` puts the binary at that path under the name `gmail`. If you built from source, the same command is `cargo run -p focusdeck-gmail -- login ...` or the built `focusdeck-gmail login ...`. If the session expires, the plugin's error says `focusdeck-gmail login`, which is the binary's name in the source tree; for the installed binary use the path above.

The command prints an address to open in the browser and, after you approve, writes the token to the `--token-file` path with mode `0600`.

## Releasing

Each plugin is released with its own tag. First bump the version in `plugins/<name>/Cargo.toml` and push it to `main`, then:

```
git tag <name>-v<version> && git push origin <name>-v<version>
```

The tag must look like `<name>-v<version>` and match the package version. The workflow builds binaries for two targets, opens a GitHub release and updates `index.toml`. Push at most three tags at once: when more than three tags arrive in one push, GitHub starts no workflow at all.

## Trying it locally

`bash scripts/local-index.sh` builds the binaries and writes `dist/index.toml`. It prints the file's path; `dist/` is not tracked by git.
