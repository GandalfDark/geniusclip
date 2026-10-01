#!/usr/bin/env bash
# Builds GeniusClip and publishes a release to the public releases repo:
#   GeniusClip-Setup.exe         the installer people download (branded window)
#   GeniusClip-<v>-update.exe    the plain NSIS setup the in-app updater runs
#   latest.json                  the updater manifest (with the signature)
#
#   scripts/release.sh "Что нового"                     build and publish
#   scripts/release.sh "Что нового" --build-only        build only
#   scripts/release.sh "Что нового" --from-ci <run id>  publish the files built
#       by the "Build release" workflow (.github/workflows/release.yml)
#
# Run from Git Bash (the updater key has an empty password, which can't be
# passed through the Windows environment, only as `-p ""`). Bump the version
# in src-tauri/tauri.conf.json, Cargo.toml and package.json first. Needs the
# GitHub CLI logged in as the repo owner and the key in ~/.tauri/geniusclip.key.
set -euo pipefail
cd "$(dirname "$0")/.."

NOTES="${1:?release notes required}"
MODE="${2:-}"
RUN_ID="${3:-}"
REPO=GandalfDark/geniusclip-releases
VERSION=$(node -p "require('./src-tauri/tauri.conf.json').version")
TAG="v$VERSION"
KEY="$HOME/.tauri/geniusclip.key"
[ -f "$KEY" ] || { echo "updater key not found: $KEY" >&2; exit 1; }
if [ "$MODE" != "--build-only" ] && gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "release $TAG already exists: bump the version first" >&2
    exit 1
fi

OUT=target/release/publish
UPDATE_NAME="GeniusClip-$VERSION-update.exe"
rm -rf "$OUT" && mkdir -p "$OUT"

if [ "$MODE" = "--from-ci" ]; then
    [ -n "$RUN_ID" ] || { echo "usage: scripts/release.sh \"notes\" --from-ci <run id>" >&2; exit 1; }
    echo "== Files from CI run $RUN_ID"
    gh run download "$RUN_ID" --repo GandalfDark/geniusclip --name "geniusclip-$VERSION" --dir "$OUT"
    for f in GeniusClip-Setup.exe "$UPDATE_NAME"; do
        [ -f "$OUT/$f" ] || { echo "the run has no $f (version $VERSION)" >&2; exit 1; }
    done
else
    # Keep local paths (and the Windows user name in them) out of the
    # binaries' panic/debug strings.
    HOME_WIN=$(cygpath -w "$HOME")
    export RUSTFLAGS="--remap-path-prefix=$HOME_WIN=~ --remap-path-prefix=$(cygpath -w "$PWD")=."

    echo "== App and NSIS setup ($VERSION)"
    npx tauri build --config scripts/tauri-release.json
    NSIS="target/release/bundle/nsis/GeniusClip_${VERSION}_x64-setup.exe"
    cp "$NSIS" "$OUT/$UPDATE_NAME"

    echo "== Installer"
    GC_SETUP_PAYLOAD="$(cygpath -w "$PWD/$NSIS")" cargo build --release -p geniusclip-setup
    cp target/release/GeniusClip-Setup.exe "$OUT/GeniusClip-Setup.exe"

    if LC_ALL=C grep -a -q -F "$HOME_WIN" target/release/GeniusClip.exe "$OUT/GeniusClip-Setup.exe"; then
        echo "local paths leaked into the binaries" >&2
        exit 1
    fi
fi

echo "== Updater signature"
# Bound to the version: a tampered latest.json can't pair a new version
# number with an older release (the app requires it, requireSignedVersion).
npx tauri signer sign -f "$KEY" -p "" --app-version "$VERSION" "$OUT/$UPDATE_NAME" >/dev/null
[ -s "$OUT/$UPDATE_NAME.sig" ] || { echo "signing failed" >&2; exit 1; }
# Apps from 0.1.9 on refuse an update whose signature names another version
# than latest.json (requireSignedVersion): they would be stuck on theirs.
SIGNED=$(base64 -d "$OUT/$UPDATE_NAME.sig" | grep -a "^trusted comment:" | tr '\t' '\n' | sed -n 's/^version://p')
[ "$SIGNED" = "$VERSION" ] || { echo "the update is signed for version '$SIGNED', not $VERSION" >&2; exit 1; }

echo "== latest.json"
node -e '
const [version, notes, sigFile, url, out] = process.argv.slice(1);
const signature = require("fs").readFileSync(sigFile, "utf8").trim();
const pub_date = new Date().toISOString().replace(/\.\d+Z$/, "Z");
require("fs").writeFileSync(out, JSON.stringify({ version, notes, pub_date,
  platforms: { "windows-x86_64": { signature, url } } }, null, 2));
' "$VERSION" "$NOTES" "$OUT/$UPDATE_NAME.sig" "https://github.com/$REPO/releases/download/$TAG/$UPDATE_NAME" "$OUT/latest.json"

ls -la "$OUT"
if [ "$MODE" = "--build-only" ]; then
    echo "Not published (--build-only). Files: $OUT"
    exit 0
fi
# The release page gets install help under the notes (the app and latest.json
# show the notes alone).
BODY="$NOTES

---

**Install:** download [GeniusClip-Setup.exe](https://github.com/$REPO/releases/download/$TAG/GeniusClip-Setup.exe) and run it (Windows 10 or 11, 64-bit). Already have GeniusClip? It offers this update by itself.
If Windows shows *\"Windows protected your PC\"*, click **More info → Run anyway** — the installer isn't code-signed yet ([why](https://github.com/$REPO#is-it-safe)).
\`$UPDATE_NAME\` and \`latest.json\` are used by the built-in updater."
gh release create "$TAG" "$OUT/GeniusClip-Setup.exe" "$OUT/$UPDATE_NAME" "$OUT/latest.json" \
    --repo "$REPO" --title "GeniusClip $VERSION" --notes "$BODY"
echo "Published https://github.com/$REPO/releases/tag/$TAG"
