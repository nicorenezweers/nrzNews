#!/bin/sh
# Paket für die App nrz-rss (Anzeigename nrzNews) im nrzStore: baut Oberfläche und Programm für
# Linux amd64 und arm64 und legt sie als neuen Stand auf den Zweig `umbrel` (nur fertige Dateien)
# mit dem Tag `umbrel-<version>`. hooks/pre-start der App lädt sie von dort (nrzGit, Port 8390).
#
#   scripts/umbrel-release.sh            # Version aus Cargo.toml
#   PUSH=1 scripts/umbrel-release.sh     # danach Zweig und Tag pushen
#   NOTES="…" …                          # Text des Tags = releaseNotes im nrzStore
#   SKIP_BUILD=1 …                       # schon gebaute Programme unter target/ nehmen
#
# Braucht: dx 0.7 (Dioxus-CLI), cargo-zigbuild mit zig, die Rust-Ziele
# x86_64-unknown-linux-gnu und aarch64-unknown-linux-gnu.
set -eu
cd "$(dirname "$0")/.."
root=$(pwd)
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
tag="umbrel-$version"

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "Tag $tag gibt es schon – Version in Cargo.toml erhöhen." >&2
  exit 1
fi

if [ "${SKIP_BUILD:-0}" != 1 ]; then
  sh scripts/ui.sh
  # glibc 2.31: läuft auf Debian ab bullseye (die App nimmt debian:trixie-slim)
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    cargo zigbuild --release --locked --target "$t.2.31"
  done
fi

out=$(mktemp -d)
trap 'git -C "$root" worktree remove --force "$out" 2>/dev/null || rm -rf "$out"' EXIT
if git rev-parse -q --verify refs/heads/umbrel >/dev/null; then
  git worktree add -q "$out" umbrel
elif git ls-remote --exit-code --heads origin umbrel >/dev/null 2>&1; then
  git fetch -q origin umbrel:umbrel
  git worktree add -q "$out" umbrel
else
  git worktree add -q --detach "$out"
  git -C "$out" checkout -q --orphan umbrel
  git -C "$out" rm -rfq . >/dev/null
fi

gzip -9 -n -c target/x86_64-unknown-linux-gnu/release/nrznews > "$out/nrznews-amd64.gz"
gzip -9 -n -c target/aarch64-unknown-linux-gnu/release/nrznews > "$out/nrznews-arm64.gz"
cp Rocket.toml "$out/Rocket.toml"
(cd "$out" && shasum -a 256 nrznews-amd64.gz nrznews-arm64.gz Rocket.toml > SHA256SUMS)
cat > "$out/README.md" <<EOF
# nrzNews – Paket für den nrzStore

Nur fertige Dateien für die App \`nrz-rss\` (nrzNews) im nrzStore: das Programm für Linux
(\`nrznews-amd64.gz\`, \`nrznews-arm64.gz\`, Oberfläche eingebettet), \`Rocket.toml\` und
\`SHA256SUMS\`. Erstellt mit \`scripts/umbrel-release.sh\` aus dem Zweig \`main\`.

Version: $version
EOF

git -C "$out" add -A
git -C "$out" commit -qm "nrzNews $version fuer den nrzStore"
git tag -a "$tag" -m "${NOTES:-nrzNews $version}" "$(git -C "$out" rev-parse HEAD)"
echo "Zweig umbrel und Tag $tag erstellt."
if [ "${PUSH:-0}" = 1 ]; then
  git push origin umbrel "$tag"
fi
