#!/bin/sh
# Sets the toolchain version everywhere it lives (proposal/releases.md §4:
# one version for CLI, LSP, embedded std, and the VS Code extension):
#
#   scripts/bump-version.sh 0.2.0
#
# Then: write the CHANGELOG section, commit, tag `v0.2.0`, push the tag —
# CI does the rest (.github/workflows/release.yml).
set -eu

VERSION="${1:?usage: scripts/bump-version.sh <version>   (e.g. 0.2.0)}"
cd "$(dirname "$0")/.."

for manifest in crates/*/Cargo.toml; do
    sed -i.bak "s/^version = \".*\"/version = \"$VERSION\"/" "$manifest"
    rm "$manifest.bak"
done
# Refresh Cargo.lock's workspace-member entries.
cargo update --workspace --quiet

# `npm version` keeps package.json and package-lock.json in step.
npm --prefix editors/vscode version --no-git-tag-version "$VERSION" > /dev/null

if ! grep -q "^## v$VERSION " CHANGELOG.md; then
    echo "note: CHANGELOG.md has no '## v$VERSION — <date>' section yet; the release workflow requires one" >&2
fi
echo "version set to $VERSION (crates, Cargo.lock, extension)"
