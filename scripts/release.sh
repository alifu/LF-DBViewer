#!/usr/bin/env bash
# Builds the universal (Apple silicon + Intel) app once, zips it, and optionally publishes it:
# a GitHub release carrying LF-DBViewer.zip, and the cask in alifu/homebrew-tap that points at it.
# People who install through Homebrew download this finished app; nothing is compiled on their machine.
#
#   scripts/release.sh             build and zip only (dist/LF-DBViewer.zip)
#   scripts/release.sh --publish   also create the release and update the cask
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="alifu/LF-DBViewer"
TAP="alifu/homebrew-tap"
APP="LF-DBViewer"
VERSION="$(node -p "require('./src-tauri/tauri.conf.json').version")"
BUNDLE="src-tauri/target/universal-apple-darwin/release/bundle/macos/${APP}.app"
ZIP="dist/${APP}.zip"
PUBLISH=false
[[ "${1:-}" == "--publish" ]] && PUBLISH=true

if $PUBLISH && [[ -n "$(git status --porcelain)" ]]; then
  echo "Commit or stash your changes first: the release is tagged from HEAD." >&2
  exit 1
fi

rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
npm run tauri build -- --target universal-apple-darwin --bundles app

# Both processor types must be inside, or half of the Macs could not open it.
ARCHS="$(lipo -archs "${BUNDLE}/Contents/MacOS/lf-dbviewer")"
echo "Architectures: ${ARCHS}"
[[ "$ARCHS" == *arm64* && "$ARCHS" == *x86_64* ]] || { echo "Not a universal binary." >&2; exit 1; }

# No Developer ID, so the app is signed ad hoc (needed for Apple silicon to run it at all).
codesign --force --deep --sign - "$BUNDLE"
codesign --verify --deep --strict "$BUNDLE"

mkdir -p dist
rm -f "$ZIP"
ditto -c -k --sequesterRsrc --keepParent "$BUNDLE" "$ZIP"
SHA="$(shasum -a 256 "$ZIP" | awk '{print $1}')"
echo "${ZIP}  sha256 ${SHA}"

$PUBLISH || { echo "Built only. Run with --publish to release ${VERSION}."; exit 0; }

if gh release view "$VERSION" -R "$REPO" >/dev/null 2>&1; then
  echo "Release ${VERSION} already exists; bump the version in package.json, tauri.conf.json and Cargo.toml." >&2
  exit 1
fi
git push origin HEAD
gh release create "$VERSION" "$ZIP" -R "$REPO" --target "$(git rev-parse HEAD)" --title "${APP} ${VERSION}" \
  --notes "Unsigned build for macOS 12 or later (Apple silicon and Intel). See the README for installing: \`brew install --cask alifu/tap/lf-dbviewer\`."

TAP_DIR="$(mktemp -d)"
trap 'rm -rf "$TAP_DIR"' EXIT
gh repo clone "$TAP" "$TAP_DIR" -- --quiet
mkdir -p "${TAP_DIR}/Casks"
cat > "${TAP_DIR}/Casks/lf-dbviewer.rb" <<CASK
cask "lf-dbviewer" do
  version "${VERSION}"
  sha256 "${SHA}"

  url "https://github.com/${REPO}/releases/download/#{version}/${APP}.zip"
  name "${APP}"
  desc "Light MySQL and MariaDB client with SSH tunnels, a table designer and a relationship diagram"
  homepage "https://github.com/${REPO}"

  depends_on macos: :monterey

  app "${APP}.app"

  zap trash: [
    "~/Library/Application Support/com.alifu.lf-dbviewer",
    "~/Library/Caches/com.alifu.lf-dbviewer",
    "~/Library/WebKit/com.alifu.lf-dbviewer",
  ]
end
CASK
git -C "$TAP_DIR" add Casks/lf-dbviewer.rb
git -C "$TAP_DIR" commit -qm "lf-dbviewer ${VERSION}"
git -C "$TAP_DIR" push -q
echo "Published ${VERSION}."
