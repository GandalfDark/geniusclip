#!/usr/bin/env bash
# Publishes site/ to GitHub Pages: https://gandalfdark.github.io
# (the public repo GandalfDark/gandalfdark.github.io; created on first run).
#
#   scripts/deploy-site.sh "What changed"
#
# Refresh the screenshots first if the app's UI changed: scripts/site-shots.mjs.
set -euo pipefail
cd "$(dirname "$0")/.."

REPO=GandalfDark/gandalfdark.github.io
MSG="${1:-Update site}"
WORK=target/site-deploy

if ! gh repo view "$REPO" >/dev/null 2>&1; then
    gh repo create "$REPO" --public --description "GeniusClip — download page"
fi
rm -rf "$WORK"
git clone -q "https://github.com/$REPO.git" "$WORK" 2>/dev/null || { mkdir -p "$WORK" && git -C "$WORK" init -q -b main && git -C "$WORK" remote add origin "https://github.com/$REPO.git"; }
find "$WORK" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
cp -r site/. "$WORK/"
touch "$WORK/.nojekyll"
git -C "$WORK" config user.name "GandalfDark"
git -C "$WORK" config user.email "334239590+GandalfDark@users.noreply.github.com"
git -C "$WORK" add -A
if git -C "$WORK" diff --cached --quiet; then
    echo "Site unchanged."
    exit 0
fi
git -C "$WORK" commit -q -m "$MSG"
git -C "$WORK" push -q -u origin main
# User-site repos publish from main automatically; make sure Pages is on.
gh api -X POST "repos/$REPO/pages" -f "source[branch]=main" -f "source[path]=/" >/dev/null 2>&1 || true
echo "Published https://gandalfdark.github.io"
