# seenboard-plugins

**English** · [Türkçe](README.tr.md)

Plugins for [Seenboard](https://github.com/omertahaoztop/seenboard). Each plugin is a separate process that talks to the core with one JSON message per line.

Seenboard was called focusdeck before. These plugins target protocol 2, speak `SEENBOARD_LANG` and install under the new names; plugin binaries from the focusdeck era (protocol 1) are not compatible and must be reinstalled.

Plugins run with your user's permissions; there is no sandbox. Only install from indexes you trust.

Plugins speak the language Seenboard passes in `SEENBOARD_LANG`; run directly they use `LC_ALL`/`LC_MESSAGES`/`LANG`. English and Turkish are included, and any other language falls back to English.

## Install

```
seenboard plugin install <name>
```

The core picks this machine's target from the `index.toml` index, downloads the binary and checks its sha256. Plugin names: `vikunja`, `redmine`, `zulip`, `whatsapp`, `gmail`. A plugin's settings go in the `config` table of its box in the profile `~/.config/seenboard/profiles/<name>.toml` (`config.toml` only holds `default_profile`, `registries`, `capture_command`, `language` and `capture_words`). Settings whose name ends in `_file` point at the file that holds a secret; its mode must be `0600`.

```toml
[[box]]
id = "redmine"
title = "Redmine plan"
plugin = "redmine"
row = 0
col = 0
config = { url = "https://redmine.example.com", key_file = "~/.config/seenboard/secrets/redmine" }
```

The keys in the tables below go in this `config` table.

Server addresses must use `https://`. Plain `http://` is accepted only for `localhost` and loopback addresses, and redirects are not followed, so a key or token never travels over an unencrypted connection or to another host.

## vikunja

Lists the open tasks of a Vikunja project; marks them done, deletes them, moves them between buckets or reorders them, and creates tasks from dropped notes.

Profile settings:

| Key | Meaning |
|---|---|
| `url` | Vikunja address, for example `https://vikunja.example.com` (`/api/v1` is appended) |
| `token_file` | File holding the API token, mode must be `0600` |
| `project` | Project title |
| `bucket` | Optional bucket title; if given, only that bucket is listed |
| `from_label_prefix` | Optional prefix of the label that carries a note's sender; defaults to `kimden:`, an empty value turns the label off |

Project and bucket titles are matched exactly first, then ignoring case.

In incoming notes, `from` becomes a `<from_label_prefix><name>` label (`kimden:<name>` by default), `labels` become labels and `links` become a links paragraph in the description. Only `http`, `https` and `mailto` links are written as links; for any other only the label is kept, as plain text. For an item coming from another box, its source is written to the description as a `seenboard-source: <plugin>:<id>` line. If labelling the new task or moving it to the bucket fails, the task is deleted again, so a retry leaves no duplicate. If the delete fails as well, the error names the task that was left behind.

A note dropped from another vikunja box is moved instead of copied when its link points at this server and the task still exists there. Otherwise a new task is created.

`done` moves the task to the view's done bucket. When the view has none, the task is marked done instead.

## redmine

Lists the open Redmine issues assigned to me. No actions.

Profile settings:

| Key | Meaning |
|---|---|
| `url` | Redmine address, for example `https://redmine.example.com` |
| `key_file` | File holding the API key, mode must be `0600` |

## zulip

Lists starred messages; the `unstar` action removes the star. Reads at most the 100,000 newest starred messages.

Profile settings:

| Key | Meaning |
|---|---|
| `url` | Zulip address, for example `https://zulip.example.com` |
| `email` | Email address of the bot or user |
| `key_file` | File holding the API key, mode must be `0600` |

## whatsapp

Lists the messages of one WhatsApp chat, meant for the "message yourself" chat, from a [GOWA](https://github.com/aldinokemal/go-whatsapp-web-multidevice) REST server. The plugin sends two requests: `GET /chat/<jid>/messages` to list and `POST /message/<id>/delete` for the `done` action, so a proxy in front of GOWA only needs to pass those two. `done` asks GOWA to delete the message for you (WhatsApp's "delete for me") and GOWA drops it from its own store. This cannot be undone. Call logs are not listed. A listing that comes back incomplete is an error. A chat with more than 2,000 messages in range is an error; `since` narrows the range. The server does not report unknown chats, so a mistyped `jid` gives an empty box.

Profile settings:

| Key | Meaning |
|---|---|
| `url` | GOWA address, for example `https://gowa.example.com` |
| `user` | GOWA Basic Auth user name |
| `key_file` | File holding the GOWA password, mode must be `0600` |
| `jid` | JID of your own chat, for example `15550001111@s.whatsapp.net`; group JIDs (`@g.us`) are rejected |
| `since` | Optional, `YYYY-MM-DD`; older messages are not listed |

```toml
[[box]]
id = "whatsapp"
title = "WhatsApp"
plugin = "whatsapp"
row = 0
col = 1
config = { url = "https://gowa.example.com", user = "seenboard", key_file = "~/.config/seenboard/secrets/gowa.key", jid = "15550001111@s.whatsapp.net", since = "2026-11-01" }
```

## gmail

Lists starred Gmail messages with read-only access. No actions. Reads at most 10,000 starred messages.

Profile settings:

| Key | Meaning |
|---|---|
| `client_file` | OAuth client JSON file downloaded from Google Cloud; `login` needs a client of type Desktop app |
| `token_file` | File the refresh token is written to and read from, mode must be `0600` |
| `api_url` | Optional, overrides the Gmail API address; defaults to Google's |

To create the token once:

```
~/.local/share/seenboard/plugins/gmail login --client-file <path> --token-file <path> [--hint <email>]
```

`seenboard plugin install gmail` puts the binary at that path under the name `gmail`. If you built from source, the same command is `cargo run -p seenboard-gmail -- login ...` or the built `seenboard-gmail login ...`. If the session expires, the plugin's error gives the `login` command with the full path of the binary that is running.

The command prints an address to open in the browser and, after you approve, writes the token to the `--token-file` path with mode `0600`. It waits 5 minutes for the approval and only accepts the answer to the sign-in it started.

## Translating

Each plugin keeps its texts in `plugins/<name>/locales/`. To add a language, copy `en.ftl` to `plugins/<name>/locales/<code>.ftl` (for example `de.ftl`) and translate the values. Keep the message ids and the `{ $name }` placeholders as they are. `cargo test` fails when a language file misses an id, has an extra one or uses different placeholders than `en.ftl`. A message missing from a language is shown in English.

## Releasing

Each plugin is released with its own tag. First bump the version in `plugins/<name>/Cargo.toml` and push it to `main`, then:

```
git tag <name>-v<version> && git push origin <name>-v<version>
```

The tag must look like `<name>-v<version>` and match the package version, which must be semver. The workflow builds binaries for two targets, opens a GitHub release and updates `index.toml`. The index tool refuses a version older than the one already in `index.toml`. The repository has immutable releases turned on and a tag ruleset on `*-v*`, so a published release cannot be changed and its tag cannot be moved or deleted. When the workflow runs again for the same tag, it keeps the published files and updates the index from them. A draft left behind by a failed run is filled again from the new build and then published. Each binary gets a build provenance attestation; check one with `gh attestation verify <file> --repo omertahaoztop/seenboard-plugins`. Push at most three tags at once: when more than three tags arrive in one push, GitHub starts no workflow at all.

## Trying it locally

`bash scripts/local-index.sh` builds the binaries and writes `dist/index.toml`. It prints the file's path; `dist/` is not tracked by git.

CI runs `cargo fmt --all --check`, `cargo test --workspace --locked` and `cargo clippy --workspace --all-targets --locked -- -D warnings` with the toolchain pinned in `rust-toolchain.toml`.
