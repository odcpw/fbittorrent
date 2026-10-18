# Native Omarchy CUA checks

Checked on 2026-10-10 using the installed `cua-driver` CLI and Hyprland plugin.
The bundled browser CUA session does not expose native APIs; the separately
installed Omarchy driver can capture native windows and deliver input.

## Widget restoration

The widget was left pointing at a temporary test wrapper that no longer existed.
This made its status polling fail and hid the bar widget while the normal app ran.
Removed only that `binary` override from the Torrents widget entry in the user's
shell settings, reloaded settings, refreshed status, and opened the popup.
The normal app continued running throughout the repair.

- [Restoration receipt](widget-restoration.json) records the restored widget entry,
  normal app state and configuration backup.
- [Actual restored widget](widget-restored-current-crop.png) shows the three real
  downloads. The crop excludes the surrounding chat.
- [Actual metadata tooltip](widget-live-metadata-tooltip-crop.png) shows the complete
  truncated status line, including ETA, flags and seeding policy.

![Restored widget](widget-restored-current-crop.png)

![Native hover tooltip for truncated metadata](widget-live-metadata-tooltip-crop.png)

## Native checks before the terminal width correction

The installed artifact had SHA-256
`127932ba417dfd4e5315c966e79f641cedfb93399b3ecdd5ac6478e71cce24d0`.

- A foreground mouse click selected RSS in the actual Ghostty app.
- In a separate Foot app with private configuration and paused local torrents,
  mouse clicks selected Shows and Torrents.
- Cursor dwell scrolled a long filename after the initial delay.
- Unpressed pointer hover scrolled another row's filename while the keyboard
  selection and Details still referred to Short.txt.
- The actual Omarchy widget displayed the complete long filename through END.txt,
  with full metadata, when its truncated title was hovered.
- Hovering the fully visible Short.txt did not display a truncation tooltip.

Relevant captures: [RSS](terminal-rss-foreground.png),
[Shows](foot-shows-foreground.png), [cursor dwell](foot-long-after-dwell.png),
[hover start](foot-unpressed-hover-start.png),
[hover dwell](foot-unpressed-hover-dwell.png),
[full filename tooltip](widget-hover-dwell.png),
[short filename](widget-short-no-tooltip.png).
Some earlier desktop captures contain surrounding local desktop content; they
remain local evidence and were not posted externally.

## Terminal width correction

The native test exposed a narrow filename column at approximately 100 terminal
columns. Torrent render and mouse geometry now share a 140-column threshold for
showing Details. The existing Search threshold remains 100 columns.
A Sol6.1 xhigh subagent ran all 21 TUI interaction tests successfully, including
rendering and pointer geometry at eight torrent widths and the Search boundary.
Only src/tui/mod.rs differs from the app sources in the previously installed
build's source receipt.

The optimized build completed successfully; existing vendor warnings remain.
The exact stripped artifact was checked in native Foot at the same 749-pixel
window width. Short.txt is fully visible and the torrent list occupies the full
width. One foreground click selected RSS. Unpressed hover then scrolled the long
name while Short.txt remained selected; RGB comparison of the native captures
confirmed the long name changed and the selected row did not.

- [21 source interaction tests](layout-tests.json)
- [Optimized build log](release-layout-build.txt)
- [Final native check receipt](native-final.json)
- [Installation and retained normal app state](layout-installation.json)
- [Isolated fixture cleanup](cleanup.json)

The installed artifact SHA-256 is
`d11c8402c381eeb137794235d94466d9556db874f94f4887174807a1ac2c4491`.
The previous binary is backed up. The normal app was not restarted, and its
configuration and saved RSS/Shows files match their pre-check hashes. The new
terminal layout applies on the next app launch. The widget repair is already live.

![Full width native filename list](foot-final-torrents-crop.png)

![Native hover scrolling while Short.txt remains selected](foot-final-hover-dwell-crop.png)

The final Foot background click was also refused with `client_not_qualified`.
The long dependency rebuild took 30 minutes 39 seconds. This is a build time
observation; no runtime performance change is claimed for the layout correction.

## Limits

The driver rejected background input for the tested Ghostty and Foot windows
with `client_not_qualified`. Successful clicks used foreground delivery; native
hover used the real desktop pointer. No driver qualification, permission mode,
keyboard layout or bundled CUA restriction was changed. The keyboard shortcut
probe was refused with `unsupported_layout`, so it is not counted as a pass.
Driver input receipts said `effect: unverifiable`; the subsequent native captures
establish which changes actually appeared.
Long action/error tooltips have not been visually exercised.
