#!/usr/bin/env bash
# Build this checkout and atomically install the user's on-demand daemon.
# For agent runs use the AGENTS.md release-lock/memory-scope wrapper.
set -euo pipefail
if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  echo 'Usage: scripts/install.sh [--prefix DIR] [--binary FILE] [--plugin link|copy]'
  echo 'Build with the locked toolchain, replace ~/.local/bin/fbittorrent atomically,'
  echo 'then run setup --magnet --plugin=link. Nothing is enabled or started at login.'
  echo 'Remove the unit with fbittorrent setup --uninstall; saved data is kept.'
  exit 0
fi
install_dir="$HOME/.local/bin"
source_binary=""
plugin_mode=link
while (($#)); do
  case "$1" in
    --prefix) [[ $# -ge 2 ]] || exit 2; install_dir="$2/bin"; shift 2 ;;
    --binary) [[ $# -ge 2 ]] || exit 2; source_binary="$(realpath -- "$2")"; shift 2 ;;
    --plugin) [[ $# -ge 2 && ($2 == link || $2 == copy) ]] || exit 2; plugin_mode="$2"; shift 2 ;;
    *) echo 'Use scripts/install.sh --help' >&2; exit 2 ;;
  esac
done
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
if [[ -z $source_binary ]]; then
  cargo build --release --locked
  source_binary="$PWD/target/release/fbittorrent"
fi
mkdir -p "$install_dir"
# Serialize the final replacement/setup; the temporary has the destination's
# filesystem, so rename is atomic even while the previous inode is executing.
exec 9>"$install_dir/.fbittorrent-install.lock"
flock -w 120 9
install_tmp="$install_dir/fbittorrent.tmp"
trap 'rm -f -- "$install_tmp"' EXIT
install -m 0755 "$source_binary" "$install_tmp"
mv -f -- "$install_tmp" "$install_dir/fbittorrent"
"$install_dir/fbittorrent" setup --magnet "--plugin=$plugin_mode"
printf 'Installed %s; daemon starts on demand.\n' "$install_dir/fbittorrent"
