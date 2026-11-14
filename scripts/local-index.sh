#!/usr/bin/env bash
# Release ikililerini derleyip dist/index.toml yazar ve dosya yolunu basar.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

cargo build --release
host=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p dist
out="$root/dist/index.toml"
: > "$out"

for manifest in plugins/*/Cargo.toml; do
  name=$(basename "$(dirname "$manifest")")
  bin="$root/target/release/seenboard-$name"
  sum=$(sha256sum "$bin" | cut -d' ' -f1)
  version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$manifest" | head -n1)
  description=$(sed -n 's/^description = "\(.*\)"$/\1/p' "$manifest" | head -n1)
  cat >> "$out" <<ENTRY
[[plugin]]
name = "$name"
description = "$description"
version = "$version"
protocol = 2

[plugin.targets.$host]
url = "file://$bin"
sha256 = "$sum"

ENTRY
done

echo "$out"
