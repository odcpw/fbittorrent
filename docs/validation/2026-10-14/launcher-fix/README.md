# Desktop and widget launch fix — 14 October 2026

The launcher used the bare window pattern `fbittorrent`. Omarchy searches both window classes and titles, then focuses the first match. A separate project terminal had “fbittorrent” in its title and matched first. This reproduced the reported failure: `fbittorrent up` said “focused” while the active window was that project terminal.

## Fix

- Use the canonical `org.omarchy.fbittorrent` identity in the Rust launcher, widget defaults and the installed Torrents menu Open/Search actions. The subagent also verified that this is a valid GTK application identity; the previous bare identity is invalid.
- Add a visible application entry at `~/.local/share/applications/fbittorrent.desktop`, launching `fbittorrent up`. The existing hidden magnet handler remains separate.
- Reload the shell widget code and install the normal release-perf build, without diagnostic features or extra Rust flags.

## Actual verification

The [verification receipt](verification.json) records installed CLI focus, the widget’s own Open IPC action, a fresh `gtk-launch fbittorrent` invocation, the visible canonical terminal window, and a running process whose executable hash matches the installed build. Repeated samples captured the widget switching focus from the other terminal to the real TUI. Close then fresh desktop reopen succeeded; closing removed the fbittorrent process and reported its engine stopped. The app was left open. This exercised the real desktop and widget action, without a native pointer-click or screenshot claim.

`desktop-file-validate` and the focused whitespace check passed. The installed qBittorrent service remains inactive, dead and disabled. Menu edits were limited to the two launch commands; the original menu file is retained beside this report.

The [installation receipt](installation.json) verifies that only `src/backend.rs` and `src/setup.rs` differ from the benchmarked Rust/TOML source map. All other 1,153 files, including engine/runtime hot paths, are byte-identical. Installed binary SHA256: `b7c6ede0880bd242c7b4ebda699eb201715af976ab2732dbaf43c98f4376b19c`. The previous measured binary is preserved. No new CPU benchmark was needed or claimed for this launcher-only correction.
