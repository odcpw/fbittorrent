# Shows inbox and actual pointer interaction

Validated and installed on 2026-10-10. This extends the earlier
[engine, queue and VPN validation](../README.md).

## Changes

- Tabs are Torrents (1), Search (2), Shows (3), RSS (4). Shows displays the
  cached unread count from every screen.
- Shows opens in All unread, combining followed shows' episodes and season
  packs, newest release first. One show's full history remains on the left.
- Inbox `m` removes the selected release; `u` restores that exact last item.
  Per-show `m` toggles read state without moving selection. Shift+M applies
  to one selected show and explains its scope in All unread.
- Source checking, incomplete history and errors remain visible. An empty
  cache during a failed or pending check is not described as caught up.
- Ordinary terminal pointer motion is enabled during actual startup.
  FrankenTUI 0.6's default mouse setup captures clicks and dragging but disables
  unpressed motion. The app opts into mode 1003 before its first render;
  FrankenTUI's terminal cleanup already resets that mode.
- Double clicks compare the actual item's identity. A refresh replacing RSS
  row zero cannot turn the next single click into activation of a new item.
- IMDb release cards, rating IDs and free-text following require a matching
  title. Live suggestions for Dark returned Dark Matter first; the matcher
  now selects Dark. Case, punctuation and an optional leading The are handled,
  with TV/year preference among matching names. Existing unchecked name caches
  are bypassed through a new cache namespace. Explicit IMDb IDs still work.

Translated aliases whose returned IMDb title differs can produce no match;
an explicit IMDb ID remains available. Movie/actor watches remain a proposal.

## Evidence

| Check | Result |
| --- | --- |
| App tests | 87 passed, no failures. [app-tests.txt](app-tests.txt) |
| Final optimized build | Completed with the matching Rust 1.98 toolchain; existing vendor warnings retained. [release-build-final.txt](release-build-final.txt) |
| Actual native terminal regression | Passed against the exact stripped artifact. Checks startup mode 1003, RSS refresh/click/read, stable targets after scrolling, Shows read/undo/download and batch scope, 80×24, help/files/settings, queue/VPN restart, shutdown and reopening. [native-terminal-shows.txt](native-terminal-shows.txt) |
| CUA clicks | Controlled the real app through a temporary localhost browser terminal, with private XDG data. Clicked RSS/Shows, exercised normal unpressed hover and marquee, verified the selected keyboard target, read/undo, batch scope, refresh from 40 to 42 RSS items, following Dark (2017), reopening and quitting. Saved state and empty app stderr were checked. [cua-final.json](cua-final.json) |
| Installation | Exact tested artifact installed, previous binary/config backed up, user config and saved histories unchanged. Three downloads, Prefer encryption and required ProtonVPN preserved. [installation.json](installation.json) |

The artifact SHA-256 is
`127932ba417dfd4e5315c966e79f641cedfb93399b3ecdd5ac6478e71cce24d0`.
[build-result.json](build-result.json) records source hashes and confirms that
source stayed unchanged during final verification.

![CUA screenshot of the final inbox with isolated test data](cua-inbox.jpg)

## Test limits and retained failures

Later checks used the separate installed Omarchy CUA Driver for the actual native
widget and terminal. See [native CUA checks and widget restoration](../native-cua/README.md).
The limits below describe the original browser CUA run.

CUA native APIs were disabled. The desktop widget and native desktop terminal
window were not clicked with CUA. The browser terminal ran the actual app,
not a replica; the separate native PTY regression exercised the same artifact.
The browser terminal does not qualify native poster rendering or widget QML.
Test data and all local downloads were separate from the user's saved state.

File selection, settings and help were clicked in the preliminary CUA build;
the exact final artifact repeated those flows in the native terminal regression.
A CUA control/screenshot timeout was recovered by reopening the same test
terminal. The pending RSS read was verified in saved state and the reopened UI.

Earlier native failures are retained in the `native-terminal-shows.*failure*`
receipts. Request-phase tracing identified the replaced-row double-click bug;
the final run verifies its correction. One earlier run observed an IPC read
failure during engine settings restart; its cause was not conclusively
attributed. The final detached run passed that transition. Earlier foreground
test/build processes also received SIGTERM; detached test wrappers avoided
that harness interruption. These failures are not discarded or called proof
of successful operation.

All temporary browser/fixture services were stopped. The user's app remains
closed, and the qBittorrent-nox user service remains inactive and disabled.
No new torrent throughput or resource-speedup claim is made for this UI change.
