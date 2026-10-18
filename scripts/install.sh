#!/usr/bin/env bash
# Build this checkout and atomically install the user's on-demand daemon.
# For agent runs use the AGENTS.md release-lock/memory-scope wrapper.
set -euo pipefail
if [[ ${1:-} == --help || ${1:-} == -h ]]; then
  echo 'Usage: scripts/install.sh'
  echo 'Build with the locked toolchain, replace ~/.local/bin/fbittorrent atomically,'
  echo 'then run setup --magnet. Nothing is enabled or started at login.'
  echo 'Remove the unit with fbittorrent setup --uninstall; saved data is kept.'
  exit 0
fi
if [[ $# != 0 ]]; then echo 'Use scripts/install.sh --help' >&2; exit 2; fi
cd "$(dirname "$0")/.."
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
cargo build --release --locked
install_dir="$HOME/.local/bin"
mkdir -p "$install_dir"
# Serialize the final replacement/setup; the temporary has the destination's
# filesystem, so rename is atomic even while the previous inode is executing.
exec 9>"$install_dir/.fbittorrent-install.lock"
flock -w 120 9
install_tmp="$install_dir/fbittorrent.tmp"
trap 'rm -f -- "$install_tmp"' EXIT
install -m 0755 target/release/fbittorrent "$install_tmp"
mv -f -- "$install_tmp" "$install_dir/fbittorrent"
"$install_dir/fbittorrent" setup --magnet
printf 'Installed %s; daemon starts on demand.\n' "$install_dir/fbittorrent"
