#!/bin/sh
# Install or update Droidloom on Ubuntu (amd64 or arm64).
set -eu

case "${1:-}" in
  --help|-h)
    printf 'Usage: sh install-ubuntu.sh\nDownloads the matching Droidloom .deb pair from the packages release and installs it with apt.\nSet DROIDLOOM_VERSION to pin a version (for example 0.1.0-47).\n'
    exit 0 ;;
  '') ;;
  *) printf 'Unknown argument: %s\n' "$1" >&2; exit 1 ;;
esac
[ "$#" -le 1 ] || { echo 'Usage: sh install-ubuntu.sh' >&2; exit 1; }

command -v apt-get >/dev/null || { echo 'This installer requires apt-get (Ubuntu or Debian).' >&2; exit 1; }
command -v curl >/dev/null || { echo 'This installer requires curl.' >&2; exit 1; }
command -v dpkg >/dev/null || { echo 'This installer requires dpkg.' >&2; exit 1; }
case "$(dpkg --print-architecture)" in
  amd64|arm64) architecture=$(dpkg --print-architecture) ;;
  *) echo 'Droidloom .deb packages require amd64 or arm64.' >&2; exit 1 ;;
esac

release=https://github.com/denialwm/droidloom/releases/download/packages
api=https://api.github.com/repos/denialwm/droidloom/releases/tags/packages
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
version=${DROIDLOOM_VERSION:-}
if [ -z "$version" ]; then
  curl -fsSL "$api" -o "$temporary/release.json"
  version=$(sed -n 's/.*"name": "droidloom-runtime_\([^"]*\)_'"$architecture"'\.deb".*/\1/p' "$temporary/release.json" | sort -V | tail -n 1)
fi
[ -n "$version" ] || { echo 'Could not find a Droidloom .deb release; set DROIDLOOM_VERSION explicitly.' >&2; exit 1; }

runtime=droidloom-runtime_${version}_${architecture}.deb
image=droidloom-image_${version}_${architecture}.deb
curl -fL --retry 3 -o "$temporary/$runtime" "$release/$runtime"
curl -fL --retry 3 -o "$temporary/$image" "$release/$image"

as_root() {
  if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi
}
printf '%s\n' "Installing Droidloom $version ($architecture)." \
  'Administrator access is needed to install system files and their Ubuntu dependencies.'
as_root apt-get update
as_root apt-get install -y "$temporary/$runtime" "$temporary/$image"
printf '\nDroidloom is installed. Run droidloomctl start as your ordinary desktop user.\n'
printf 'To start it with your graphical session: systemctl --user enable droidloom.service\n'
