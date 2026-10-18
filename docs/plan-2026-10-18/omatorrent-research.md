# OmaTorrent research: two Omarchy torrent projects with the same name

Date: 2026-10-18. Audience: the fbittorrent maintainer.
Method: both repositories were cloned without file contents (`--filter=blob:none`) into the session scratchpad. The clones were read only; nothing in them was built or run. GitHub PRs, issues and the Omarchy plugin marketplace threads were read with `gh`. A web search limited to 2025-2026 material is summarised in section 1.4. Neither repository has a tag or a release, so every finding is pinned to the `main` commit given below.

| Repo | Commit read | Commit date |
|---|---|---|
| github.com/Cuciz/omatorrent | `e35b03f5a71a7b7c640b8a48cadfb27e1b4cc3c3` | 2026-09-20 12:56 +0200 |
| github.com/ucmz851/omatorrent | `28b967168a5b492dc31cd7d88a3514678b9db71f` | 2026-09-02 07:36 +0500 |

Code paths below are relative to each repo root and use the form `file:line`.

---

## 0. TL;DR

- **Cuciz/omatorrent** is now called **Sprout** ("Torrent client for Omarchy"). It is a qBittorrent front end made of three parts: a Go daemon that holds all state and logic, a Quickshell bar widget with a popout panel, and a separate dashboard overlay plugin. The daemon talks to qBittorrent's WebUI API and serves the shell over NDJSON on a Unix socket. It was built in about 38 hours, from 2026-09-18 to 2026-09-20, as 6 phase PRs. It has a lot of architecture and security work but few features: list, filter, pause/resume, add a magnet, remove with or without files, a remote connection with TLS pinning, and a dashboard. It is not in the marketplace. Advanced controls are the open next phase (#14).
- **ucmz851/omatorrent** is a **search-first** bar widget. A Python stdlib helper that QML launches as a subprocess searches user-configured indexers, and a second tab drives a localhost qBittorrent WebUI with no authentication. It has more qBittorrent controls than Sprout today: force start, recheck, global, alternative and per-torrent speed limits, and editing the default save path. Its architecture and safety are weak: it polls by forking processes, has no delete confirmation and uses hard-coded colours. It was listed in the marketplace on 2026-08-28 and delisted the same day by a marketplace maintainer (PR #3028, "not a security classification"). A resubmission (#3231) was closed on 2026-09-16 with fixes still requested. Across both tickets there were 7 security review rounds (2 on #2005, 5 on #3231).
- **Neither project claims "feature parity".** The word does not appear in either repo, PR or marketplace thread. "qBittorrent front end" fits **Cuciz/omatorrent (Sprout)**. Its roadmap expressly rejects becoming "a dense qBittorrent clone" (issue #14) and puts RSS and search out of scope. The "aiming for feature parity" part is an overstatement, wherever it came from.

---

## 1. Identification

### 1.1 Side by side

| | Cuciz/omatorrent ("Sprout") | ucmz851/omatorrent ("OmaTorrent") |
|---|---|---|
| GitHub description | "OmaTorrent - native Omarchy torrent control: Quickshell UI + Go daemon over the qBittorrent WebUI API" | "High-speed multi-indexer torrent search engine and magnet dispatcher for Omarchy Quattro bar" |
| Current public name | **Sprout**, tagline "Torrent client for Omarchy". Renamed in PR #13. Technical IDs still say `omatorrent` (docs/BRAND.md) | OmaTorrent (manifest `name`) |
| Plugin IDs | `local.omatorrent` (bar widget) and `local.omatorrent-dashboard` (overlay). Both are dev-namespace IDs; the public ID is still open (docs/PRODUCT.md) | `ucmz851.omatorrent-client`. It was `ucmz851.omatorrent` until commit 650067a |
| Version in manifest | 0.5.1 | 1.0.0 |
| Created / last push | 2026-09-18 / 2026-09-20 | 2026-08-23 / 2026-09-02 |
| Commits / contributors | 74 / 1 (Cuciz). PRs and review rounds are self-authored; the repo carries an AI-agent dev harness (`tools/zcode-marketplace/...`, 6 agent roles) | 25 / 1 (Usama Imran Cheema) |
| Tags / releases | none / none | none / none |
| Stars / forks | 0 / 0 | 1 / 0 |
| PRs | 6 merged (#2, #4, #6, #8, #10, #13) | none; everything was pushed to main |
| Open issues | #12 (stale panel after backend switch: investigated, not reproduced, hardening applied) and #14 (Phase 0.6 advanced controls) | none (#1, about the screenshots, closed) |
| License | "MIT (intended; confirm before first release)". No LICENSE file | MIT, LICENSE file present |
| Code size | Go: 6,444 source lines + 7,581 test lines. QML/JS: 3,221. Docs: 4,969 lines of markdown | Panel.qml 2,034, BarWidget.qml 92, scripts/torrent_engine.py 1,320 |
| Marketplace | Never submitted. No marketplace issue mentions Sprout or Cuciz | #2005: listed and verified 2026-08-28. A maintainer delisted it the same day and retired the ID (marketplace PR #3028, "not a security classification"). #3231: resubmitted under the new ID, labelled `needs-fixes` and `security-needs-fixes`, closed 2026-09-16 for no response within 7 days |
| Maturity | Pre-alpha ("Phase 0.5.1"). The core is well tested but the feature set is small. No commits since 2026-09-20 (four weeks before this report); #14 is open with no branch pushed | Functional but unreviewed. Inactive since 2026-09-02 |

### 1.2 Which one the maintainer most likely means

The description "OMAtorrent is a qBittorrent front end aiming for feature parity" fits **Cuciz/omatorrent**:

1. Its repo description is literally "Quickshell UI + Go daemon over the qBittorrent WebUI API". The whole product definition is "a control plane: its engine is qBittorrent" (docs/PRODUCT.md). ucmz851's own description leads with "multi-indexer torrent search engine and magnet dispatcher". qBittorrent is a secondary tab there and is optional: without it, magnets fall back to `xdg-open`.
2. The PRs and issue named in the research brief (#2, #8, #10 and issue #3) all belong to Cuciz.
3. Its roadmap climbs toward a large qBittorrent subset. Phase 0.6 adds advanced controls, 0.7 adds file tree and priorities, and 0.8 adds metrics and history (docs/ROADMAP.md).

**But "feature parity" is not the project's goal.** Issue #14 says 0.6 must deliver the advanced controls "without turning Sprout into a dense qBittorrent clone". It lists RSS, search and automation as out of scope. docs/QBITTORRENT.md marks peers, file/folder rename and many add-dialog options "NOT NEEDED BEFORE 1.0". The maintainer should read Sprout as a *native Omarchy control surface over qBittorrent*, not a qBittorrent replacement.

### 1.3 Other torrent plugins in the same marketplace (context)

A search of the marketplace issue tracker for "torrent" and "qbittorrent" turns up several other listings. They show the space is crowded: `OmaqBT` by Aweiward (#364, listed; Sprout's docs/QUICKSHELL.md names it as prior art), `Torrents`/`widget.torrents` by Xfedis (#3864, listed, `manual-setup`), `Torrents` by andershfranzen (#6538, closed `needs-fixes`), `omARR` (#3094, listed) and `Omarchy Proton` (VPN with a "Torrent Tunnel", #9451, listed). Section 1.4 has details.

### 1.4 Web search (2025-2026)

A background agent searched the web, Reddit (blocked for that tool), X, plugins.omarchy.org (`catalog.json`, generated 2026-10-18; omarchyplugins.com now 301-redirects there), the marketplace registry and omarchystore.com. Every relevant hit is from 2026. No 2025 Omarchy torrent-plugin material exists.

- **The phrase "feature parity" does not appear in any public source** about any Omarchy torrent plugin: no repo, README, manifest, release, issue, PR, marketplace thread or catalog entry uses it. The only qBittorrent "feature parity" wording found online is on the qBittorrent wiki page about alternate WebUIs (https://github.com/qbittorrent/qBittorrent/wiki/List-of-known-alternate-WebUIs), which has nothing to do with Omarchy. The maintainer's sentence is best treated as a paraphrase, not a quote.
- **Why ucmz851 was delisted:** marketplace PR **#3028 "Delist OmaTorrent"** (HANCORE-linux, merged 2026-08-28 10:52Z) removed the listing, retired the ID `ucmz851.omatorrent` so that future submissions fail closed, and deleted its preview assets. The PR says: "This is a marketplace maintainer delisting, not a security classification of the upstream project." The author's question on #2005 ("Why the plugin has been removed") came later that day. The resubmission under a new ID (#3231) then hit the compiled-in-indexer policy question and the security review rounds described in gotcha 6.7.
- **Projects closer to "parity" in practice** (none of them claims parity):
  - **Aweiward/OmaqBT** (`aweiward.omaqbt`, v2.1.3, 10 stars, listed, manual setup): "qBittorrent in the Omarchy bar, with a quick popup and a full window for search, RSS and settings". It drives a local qbittorrent-nox, can install nox via pkexec, and binds to Mullvad. Its verification thread #9625 fixed a localhost-auth bypass and passkeys leaking through argv. A fork, manuelseeger/omaqbt, adds a remote mode and is not listed. omacom/omarchy issue #14672 (2026-10-09) lists it among plugins broken by a Qt 6.12 Color change.
  - **diogochaves/omaseed** (https://omaseed.app; v0.2.0 on 2026-10-13; 17 stars): a native C++/Qt 6 qBittorrent client for Omarchy with a bar widget, multi-server support and seed history. It is not a marketplace plugin.
- **Other torrent listings:**
  - **Listed**: `widget.torrents` by Xfedis (monitors Transmission, qBittorrent and Deluge; standard-install request #11942 is still open); `marcuspelo.omarqui` (speed and status through Qui); omARR; Omarchy Proton (qBittorrent inside its own WireGuard tunnel); `panda-dl`; aria2 "Downloads" and Tugboat.
  - **Not listed**: `anders.torrents` (an aria2 client; #6538 closed over an unbounded RPC read), ucmz851 OmaTorrent (delisted and the ID retired; resubmission closed) and Cuciz Sprout (never submitted).
- **Media:** no videos or posts were found for either OmaTorrent. The only images are the in-repo screenshots described in section 3.

---

## 2. Architecture

### 2.1 Cuciz/omatorrent (Sprout)

**Process model** (docs/ARCHITECTURE.md, ADR-0001):

```
omarchy-shell (one long-lived Quickshell process: quickshell -n -p /usr/share/omarchy/shell)
 ├─ local.omatorrent            kinds:["bar-widget"]  BarWidget.qml + Panel.qml (popout)
 └─ local.omatorrent-dashboard  kinds:["overlay"]     Dashboard.qml (layer-shell overlay)
        │  each surface opens its OWN Unix-socket connection
        ▼  $XDG_RUNTIME_DIR/omatorrent/service.sock   NDJSON, protocol 1 (v1.0-v1.4)
omatorrent-service (Go 1.27, stdlib only, systemd user unit, ~/.local/bin)
 ├─ internal/ipc         socket lifecycle + strict parser + per-connection writer goroutine
 ├─ internal/state       Syncer: sync/maindata rid loop, normalized state, epochs, Aggregate()
 ├─ internal/mutate      staged mutations, ref replay ring, state-derived confirmation
 ├─ internal/qbittorrent the ONLY qBittorrent-aware package (adapter)
 ├─ internal/connection  profile (connection.json), URL/TLS policy, configure transactions
 ├─ internal/secrets     Secret Service through the `secret-tool` subprocess
 └─ internal/config      service.json (0600 enforced)
        ▼  HTTP(S)
qBittorrent WebUI API v2 (verified against qbittorrent-nox 5.2.3 / WebAPI 2.15.1)
```

QML is presentation only. It makes no HTTP calls, holds no secrets and does no merge logic (ADR-0001). The decision record gives the reasons: if logic lived in the shell, a logic bug would become a desktop crash, state would be lost on a shell reload, and tests would need a running shell.

**IPC protocol** (docs/IPC.md; ADRs 0003-0008; `omatorrent-service/internal/ipc/protocol.go`):

- Transport: a Unix stream socket. The app directory is 0700 and the socket 0600. The runtime directory is validated: it must be absolute, owned by the user, mode 0700 and contain no symlinks. The daemon removes a stale socket only after proving the old listener is dead (ECONNREFUSED, exact file shape, an identity re-check, and a hello probe). Any ambiguous state makes it refuse to start.
- Framing: each message is one UTF-8 JSON object plus LF, **at most 4096 bytes** (`const MaxFrame = 4096`, protocol.go:22). Exact key sets are enforced. Unknown fields, duplicate keys, nulls, non-integer ids, invalid UTF-8 and trailing JSON are all rejected as `invalid_message`, and the connection is closed. Error frames never echo the payload or the id.
- Versioning: an integer major version in the handshake, `{"type":"hello","protocol":1}`. Minor versions only add **new message types** and never add fields to existing shapes, so a strict v1.0 client keeps working.
- Limits: 16 clients, a 5 s handshake deadline, a 30 s idle read deadline (a client that only listens gets dropped; issue #12) and a 5 s write deadline.
- Client discipline: **one request in flight per connection**, responses matched by `id`, and a 6 s stuck-response watchdog. After that the client resets the session and reconnects with backoff from 1 s to 10 s.

| Version | Messages | Notes |
|---|---|---|
| v1.0 | `hello`, `health`, `system.status` | Status comes only from the daemon cache; answering never touches qBittorrent |
| v1.1 | `torrent.subscribe` → `torrent.subscribed`, `torrent.snapshot.begin/item/end`, `torrent.delta{seq,changed[],removed[]}` | Snapshot delivery uses backpressure (30 s window). Live deltas go through a bounded 256-frame queue, and overflow disconnects the client, which then rebuilds from a fresh snapshot |
| v1.2 | `torrent.pause/resume/add/remove` (+ a `ref` idempotency key) → `mutation.accepted` or `mutation.rejected{code}` → pushed `mutation.result{confirmed|timeout}` | "Accepted ≠ confirmed". Confirmation is derived from synced state within a 10 s window. `delete_files` is a **required boolean**. Limits: 4 concurrent submissions per daemon and a 64-slot ref replay ring |
| v1.3 | `dashboard.status` | Live shape: speeds, `free_space`, `counts`, `aggregate`, and `active[]` with at most 5 entries. Degraded shape: only `last_known`, never fabricated zeroes |
| v1.4 | `connection.status/test/configure` | `test` probes for at most 8 s with a throwaway client. `configure` changes the backend epoch and sends `secret_action: keep|replace|delete` |

Torrents cross IPC in a **normalized** form: hash, name (at most 512 runes), state, progress, dlspeed, upspeed, eta, ratio, category, size and completed. qBittorrent's ~20 states are collapsed to 8 (`downloading, seeding, paused, queued, checking, error, moving, other`; `internal/state/syncer.go:666`).

**qBittorrent coverage.** All of it lives in `internal/qbittorrent/client.go`, and this is the complete list:

| Endpoint | Use |
|---|---|
| `POST auth/login`, `POST auth/logout` | Login result handling depends on the version: 5.2 answers 204 or 401, while ≤5.1 answers `Ok.` or `Fails.`. 403 means banned. The client sends no Origin/Referer, keeps cookies in a jar and never assumes the cookie name (`QBT_SID_<port>` on 5.2, `SID` before). Logout happens on a backend switch |
| `GET app/version`, `GET app/webapiVersion` | Probed once per epoch. The version decides stop/start versus pause/resume |
| `GET sync/maindata?rid=N` | The only state source. Polled every **2 s**; on failure it backs off from 2 s to 30 s. Auth failures back off from 30 s to 10 min, and after **3** bad-credential logins the state becomes a sticky `auth_failed`, below qBittorrent's 5-attempt / 1 h IP ban (`syncer.go:175`, `:288-330`) |
| `POST torrents/stop`, `torrents/start` (fallback `pause`/`resume` when WebAPI < 2.11.0) | Always one hash and never `all` |
| `POST torrents/delete` | `deleteFiles` is always sent explicitly |
| `POST torrents/add` (`urls` = a magnet only) | The daemon checks the magnet structure itself, cross-checks the hash qBittorrent echoes back (5.2 JSON response), and maps 409 to `duplicate` or `backend_rejected` |

Eleven endpoints in total, counting the legacy pause/resume fallback. There is no `torrents/info`, files, trackers, peers, categories, tags, transfer limits or preferences. Requests never follow redirects (`http.ErrUseLastResponse`) and every body read is capped: 64 MiB for sync data, 256 KiB for POSTs, 64 KiB for login.

**State handling** (`internal/state/syncer.go`). The syncer owns `rid`. On `full_update` it rebuilds and diffs against the previous state, so removals are published and no ghost rows remain. Partial delta objects are merged field by field. A malformed payload leaves the last good state in place. Each commit gets a generation number, and the copy-on-write map is cloned per cycle (O(N + delta)). Measured cost at 1,000 torrents on an i5-1334U: a full rebuild takes 8.09 ms and a 10-change delta 375 µs (PR #4). A **backend epoch** isolates backend switches: commits from a stale epoch are discarded, the old torrents are published as removals, and mutations in flight make `configure` fail with `mutations_pending`. That last rule means a mutation can never be applied to the wrong backend.

**Security hardening** (docs/SECURITY.md, ADR-0008/0009, PR #10):

- Credentials live only in the Secret Service, reached through `secret-tool` with the secret on stdin/stdout and never in argv. The child gets a minimal environment, and the item label is ASCII (see gotcha 6.3). There is no plaintext fallback, and the legacy `password` field in `service.json` fails to load.
- URL policy: only http and https; no userinfo, query or fragment; any percent-encoding in a base path is refused. Remote plain HTTP requires an explicit, persisted `allow_insecure_http`, and status reports the *factual* `insecure` flag. TLS verification cannot be turned off at any layer; the options are system roots, an explicit CA bundle, or **TOFU certificate pinning** (the SHA-256 of the leaf certificate crosses IPC, the certificate itself never does).
- Config files must be 0600 and owned by the user. Reads use `O_NOFOLLOW` plus `fstat`. Writes create an exclusive temp file, fsync it and rename it atomically. A file with permissive mode fails closed and is not repaired.
- `configure` is transactional. Snapshots of the raw profile bytes and the previous secret are taken *after* acquiring exclusivity, and rollback restores only what actually changed (see gotcha 6.2).

**Quickshell / Omarchy integration** (docs/QUICKSHELL.md and the QML):

- Bar: `BarWidget` + `WidgetButton` from `qs.Ui` with `labelVisible:false`. A custom `Row` draws a Canvas glyph followed by the label, sized to `button.fontSize` so the bar never gets taller. Colour changes animate only when `bar.foregroundAnimationEnabled` is set (`BarWidget.qml:252-320`).
- Panel: extends `Ui.Panel` (`manageIpc:false`) and renders in `KeyboardPanel` (`anchorItem` = the bar button, `contentWidth: Style.space(360)`, content-driven height) with `PanelKeyCatcher` for close-on-focus-loss (`Panel.qml:715-733`). This copies the first-party `clock` popout. The widget exposes `open/close/toggle/opened`, so `omarchy-shell shell toggle local.omatorrent` works.
- Dashboard: a **separate overlay-kind plugin**. Adding `overlay` to the bar-widget plugin's kinds would make `isBarWidgetPanelPlugin()` false and send toggles to the wrong place. The overlay is a full-anchor `PanelWindow` (`WlrLayershell.layer: Overlay`, `keyboardFocus: Exclusive` while open, `exclusionMode: Ignore`, namespace `omatorrent-dashboard`). It draws a `Color.menu.scrim` scrim and a centred `BorderSurface` card (`Color.menu.background`, `Border.surfaceSpec("menu", ...)`, `Style.cornerRadius`) (`Dashboard.qml:299-330`). It is **not** `keepLoaded`: closing destroys the component together with its socket and timers. A 50x open/close test left the daemon's fd count flat.
- Navigation between surfaces goes through the host CLI. The panel opens the dashboard with `bar.run("omarchy-shell shell toggle local.omatorrent-dashboard")`. The dashboard opens the panel with `Util.execDetached("omarchy-shell shell summon local.omatorrent")`, because the plugin-scoped shell API cannot summon *other* plugins.
- Packaging: `packaging/systemd/omatorrent-service.service` (`ExecStart=%h/.local/bin/omatorrent-service`, `Restart=on-failure`, `NoNewPrivileges`, `PrivateTmp`). Flags are `-config`, `-socket` and `-connection`; the env var `OMATORRENT_CONNECTION` sets the connection path. The `ot-probe` debug client takes `-socket`, `-interval` and `-count`. Runtime dependency: `libsecret` (`secret-tool`).
- Development process: the repo ships a "ZCode" agent harness with architect, backend, quickshell, qbt-researcher, security-reviewer and qa-release agents. Two screenshots (`docs/screenshots/phase03-panel-live.png`, `phase05-panel-tls-untrusted.png`) catch the workflow in action: ChatGPT drafted ASCII UI mockups and reviewed the work, and a GLM-5.3 coding agent implemented it. Code comments refer back to those mockups ("ASCII D", "ASCII F/G", "ASCII I").

### 2.2 ucmz851/omatorrent

**Process model: no daemon.** `BarWidget.qml` loads `Panel.qml`. The panel runs `python3 scripts/torrent_engine.py ...` as Quickshell `Process` objects and collects stdout with `StdioCollector`:

| Process | Trigger | argv |
|---|---|---|
| `searchProc` | Enter in the search field | `--query Q --category C --provider P --sort S` |
| `qbPollProc` | **Every 2.5 s, only while the panel is open** (`qbAutoTimer`, `Panel.qml:205-211`), on open, and after every action | `--qbittorrent PORT` (`0` = auto-detect) |
| `qbActionProc` | Any control | `--qb-action ACTION HASH EXTRA PORT` |
| `indexersProc` | On open | `--list-indexers` |

Each poll forks a new Python interpreter, which makes **5 HTTP calls**: `app/version`, `transfer/info`, `app/preferences`, `transfer/speedLimitsMode` and `torrents/info?filter=all` (`torrent_engine.py:592-660`). It does not use `sync/maindata`. Because polling stops when the panel closes, the bar icon's colour and tooltip go stale until the panel is opened again (`BarWidget.qml:42-50`).

**qBittorrent coverage** (`control_qbittorrent`, `torrent_engine.py:779-850`): `torrents/stop|pause`, `start|resume` (it tries the 5.x endpoint, then falls back to 4.x on 404 or 400), `delete` (`deleteFiles`), `add` (`urls`, optional `savepath`), `transfer/setDownloadLimit|setUploadLimit`, `transfer/toggleSpeedLimitsMode`, `app/setPreferences` (`save_path`), `torrents/setDownloadLimit|setUploadLimit`, `setLocation` (implemented but not wired to the UI), `setForceStart` and `recheck`. **There is no authentication**: the code never calls `auth/login`. The README tells users to enable "Bypass authentication for clients on localhost". The host is hard-coded to `127.0.0.1`. The port is auto-detected by reading `WebUI\Port` from `~/.config/qBittorrent/qBittorrent.conf`, then probing `[8080, 8085, 9091, 8000, 8090, 8888, 6881, 8081]` with a 0.35 s timeout (`torrent_engine.py:555-590`). The list includes 6881, the default BitTorrent listen port.

**Search engine**: indexers come from `~/.config/omarchy/omatorrent_indexers.json`. The supported types are `archive_org`, `rss`, `torznab` (Jackett or Prowlarr, with an `apikey`) and `json_api`. The defaults are Internet Archive and LinuxTracker. Queries run in a `ThreadPoolExecutor` with at most 8 workers (`search_all`, `:1144`). Results are deduplicated and sorted by seeds, size or date, then capped at 50.

**Hardening, added while negotiating with the marketplace reviewer** (#2005, #3231):

- Bounded reads: `read_bounded` (`:80`) caps indexer responses at 1 MB and qBittorrent responses at 2 MB.
- Bounded stdout: `emit_bounded_json` (`:126`) caps output at 256 KB for search, 512 KB for qBittorrent data and 32 KB for actions. QML repeats the same ceilings.
- Field caps: titles at most 250 characters and magnets at most 2048.
- Indexer config writes go through a held directory-FD chain (`open_directory_chain`, `:360`) with an exclusive temp file, fsync and `renameat` (`safe_write_private_json`, `:422`).
- HTTPS is required when an API key is set; loopback is exempt (`validate_url_transport`, `:262`).
- A same-origin redirect handler blocks cross-origin redirects (`:282`).
- QML only uses `Text.PlainText` and always passes argv arrays to processes.

The last review still asked for more: fail-closed handling of the config path, enforced permissions, detection of credentials embedded in URLs, aggregate memory and time limits, process teardown, and a delete confirmation.

**Quickshell integration**: a `BarWidget` with `BarIconButton` (`slotSize: Style.bar.statusSlot`; Nerd Font glyph 󰚌). Middle-click refreshes. The panel is a `KeyboardPanel` with `focusTarget: searchInput`, `fittedContentWidth(Style.space(540))` and `fittedContentHeight(..., Style.space(700))`, plus `PanelKeyCatcher` with `onTabRequested: switchPanel(direction)` (`Panel.qml:352-377`). It uses `BorderSurface` with `Border.controlSpec(...)`, `Style.selectedFillFor` and `Style.hoverFillFor`. There are no overlays and no separate windows.

---

## 3. UI inventory

### 3.1 Sprout (Cuciz)

**Bar widget** (`plugins/local.omatorrent/BarWidget.qml`). A pixel "sprout" glyph (Canvas, theme foreground) is followed by a label, and there are four states:

| State | Label | Colour |
|---|---|---|
| Daemon not running | `qBT OFFLINE` | `Color.muted` |
| Daemon up, qBittorrent unreachable | `qBT ERROR` | `bar.urgent` |
| Connected, idle | `qBT ●` | muted |
| Transferring | `↓ 1.2 MB/s  ↑ 40 kB/s` | `bar.barForeground` |

The tooltip reads "qBittorrent v5.2.3 (WebAPI 2.15.1) — 3 torrents". Clicking toggles the panel. On vertical bars only the text is shown. In `docs/screenshots/phase051-bar.png`, the widget sits left of the coffee, moon and Bluetooth icons as a dim grey glyph plus "qBT ●", the same weight as the first-party icons.

**Panel** (popout under the bar button; `Panel.qml`; screenshots `phase051-panel-*.png`, 534 px wide at theme scale 1.5):

1. *Identity row*: compact glyph plus lowercase "sprout" on the left, and global "↓ 0 B/s ↑ 0 B/s" on the right in caption size.
2. *State row*: a coloured dot (accent when OK, urgent otherwise), then the connection state text: `connected`, `connected · 192.168.1.141:8090`, `connected (insecure) · host`, `authentication failed`, `certificate hostname mismatch`, `credential store locked`, and so on. On the right are three text glyph buttons: ⚙ settings (it turns into ←), ▤ dashboard and + add magnet (it turns into ×).
3. *Attention banner*, shown only for auth, TLS or secret-store failures: one urgent sentence ("The certificate is not valid for this host. The connection was NOT downgraded.") plus an accent "Connection settings" link.
4. *Insecure badge*: "insecure connection — traffic is not encrypted" in urgent.
5. *Filter row*: text tabs `All Active Downloading Seeding Paused Completed`, with the active one in accent (`Panel.qml:92`).
6. *Add row*: a `TextField` (placeholder `magnet:?xt=urn:btih:…`, background `Color.muted`, radius `Style.space(2)`) and an "Add" link. Enter submits. Only magnets are accepted.
7. *Torrent list*: a `ListView` capped at `Style.space(340)` that sizes to its content and scrolls when it overflows. Each row is `Style.space(13)` high and shows the name (elided), the state word, then either "↓/↑ speeds" or "ETA ratio" when idle (`∞ 1.17`), and a thin progress track below in accent (muted when complete). **Pause/resume and remove appear only on hover** (▶ / ‖ in accent, ✕ in urgent). While a mutation is pending, the state word changes to `pausing…` / `resuming…` / `removing…` / `adding…`. Rows are sorted by name and cannot be re-sorted. In the published 0.5.1 screenshots the per-row state word is not visible; only the name, ∞ and the ratio show.
8. *Removal confirm strip*: `Remove "<name>"?`, then **Remove** (accent), **Remove + delete files** (urgent) and **Cancel** (`Panel.qml:1197-1250`).
9. *Empty and degraded overlays*: the full glyph, a green `#A1D06A` pixel wordmark "SPROUT", "No torrents growing yet." and "Add magnet" (`phase051-panel-empty.png`). Other overlay messages: "Nothing in this filter", "omatorrent-service offline", and "qBittorrent unreachable — showing last known state" (the last-known rows stay visible underneath).
10. *Settings mode* (the ⚙ swaps the whole body): title "sprout · qBittorrent connection", then Address, Username and Password fields. The password field is write-only, with placeholder "unchanged" or "none (localhost bypass)". Below that: "stored securely" with a Forget/Keep toggle, a fixed "✓ Verify TLS certificate" line, and, when a test reports an untrusted certificate, "offered certificate <sha256>" with **Trust this certificate** (pin). When the address is remote plain HTTP: **Use HTTPS instead** / **Continue knowingly**. Then the test result ("✓ Connection successful — qBittorrent v5.2.3 WebAPI 2.15.1 HTTPS authenticated" or a specific failure) and **Test connection** / **Save**.
11. *Mutation banner*: a transient urgent line that clears after 6 s, for example "Torrent already added" or "Action result unknown — state will tell".

Keyboard: Escape and focus loss close the panel through `PanelKeyCatcher`. Enter in a text field submits it. There is **no keyboard navigation of the list** (no `Keys` handlers in Panel.qml); every action is a mouse click on a text glyph.

**Dashboard overlay** (`Dashboard.qml`; `phase051-dashboard-*.png`, an 880×950 capture). A centred card up to `Style.space(500)` wide over a scrim, made of sections with `PanelSectionHeader`:

- *Header*: glyph plus "Sprout", the tagline "Torrent client for Omarchy", and on the right the state, "qBittorrent v5.2.3 · WebAPI 2.15.1" and the remote host label, with a dot.
- *Degraded callouts*: tinted boxes (`Util.alpha(urgent, 0.10)` fill, 0.35 border), for example "Insecure connection — this backend runs acknowledged plain HTTP" or "qBittorrent unreachable — showing last known state".
- *Live transfer*: ↓ Download, ↑ Upload, and "N torrents transferring now".
- *Torrents*: Total, Downloading, Seeding, Paused, Completed (values dimmed when last-known).
- *Current data*: Total size, Downloaded, Remaining, "Free space (default save path)", a full-width completion track and "99% of all data downloaded".
- *Transferring now*: up to 5 name and speed rows.
- *Footer*: "Connection settings", "Open torrent panel" and "Esc to close".

Escape or a click on the scrim closes the overlay. In the screenshots the right-aligned values are clipped by the card edge ("11.5 GB" shows as "11.5 GE"). In the remote light-theme capture, "Free space" reads **"-1 B"** (see gotcha 6.6).

Theming: everything comes from `qs.Commons` tokens (`Color.accent/muted/urgent`, `Color.menu.*`, `Style.space()`, `Style.font.body/caption/title`, `Style.spacing.panelPadding`, `bar.barForeground`, `bar.urgent`, `bar.fontFamily`). The one hard-coded colour is the brand green on the empty-state wordmark. Dark and light themes were both verified with screenshots.

### 3.2 OmaTorrent (ucmz851)

**Bar**: a single Nerd Font icon 󰚌 whose colour is set by `getStatusColor` (`BarWidget.qml:42`): accent while open, **hard-coded** `#e5c07b` while searching, and `#87c095` when qBittorrent has active downloads or search results exist. The tooltip shows "OmaTorrent: N downloading (speed)" or "N results for 'query'". Left-click toggles the panel; middle-click refreshes.

**Panel** (`Panel.qml`, up to `Style.space(540)` × `700`):

1. *Header*: glyph, "OmaTorrent", the subtitle "Search · Magnets · Live qBittorrent Transfers", and a status chip with the version.
2. *Tab pills*: "Search Indexers" and "Transfers (N 󰜮)".
3. *Search tab*: a search box that takes focus on open; Enter searches. Below it:
   - category pills: All, Movies, TV Shows, Games, Anime, Software, Music;
   - a provider dropdown: "All Indexers (Aggregated)" or a single indexer, plus a "Configure Indexers (JSON)" entry that opens the file with `xdg-open`;
   - a sort dropdown: Most Seeds, Largest Size, Smallest Size, Newest Date;
   - result cards: provider badge, category badge, title, size, seeds (green), leechers (red), date, and actions **copy magnet** (`wl-copy`) and **send** (qBittorrent `add`, or `xdg-open` when qBittorrent is disconnected);
   - a "Querying multi-indexers…" spinner and empty/no-result states.
4. *Transfers tab*:
   - a status header: connected/version and a "Full Speed / Alt Limit ON" turtle toggle;
   - Download and Upload speed tiles;
   - a refresh button;
   - a save-path row: Open Folder, and an inline "New Path:" editor that calls `setPreferences`;
   - "Limits (DL: … · UP: …)": a dropdown of preset chips (DL: Unlimited, 1, 5, 10, 25, 50 MB/s; UP: Unlimited, 250 KB/s, 500 KB/s, 1, 2, 5 MB/s);
   - one **card per torrent** (at most 100). Each card shows a state badge (Downloading, Seeding, Paused, Stalled DL, Queued, Checking, Error, Completed), progress %, ratio, a DL-limit badge, then pause/resume, options and **delete (immediate)**. Below that: name, progress bar, ↓/↑ speed, "completed / size" and ETA.
   - The options drawer shows the path with Open Folder, per-torrent DL and UP preset chips, a "Force Start / Forced: ON" toggle, "Recheck" and **"Delete with Files" (immediate, no confirmation)** (`Panel.qml:1638-1875`).
5. *Disconnected state*: setup instructions, a port `TextInput`, and **Auto-Detect** and **Connect** buttons.

Keyboard: the search field gets focus on open, Enter searches, and Escape is layered: first it closes a dropdown, then it clears the search, then it closes the panel (`Panel.qml:366-375`). Tab switches panels through `PanelKeyCatcher.onTabRequested`. As in Sprout, there is no keyboard navigation of the result or torrent lists.

Theming: the panel uses `Color.accent` and `bar.foreground`/`urgent`, with alpha-derived `dim`/`subtle` shades. It also uses **many hard-coded One-Dark-style hex colours** for state and provider badges and speeds (`#87c095`, `#6aa6b2`, `#e5c07b`, `#d19a66`, `#c678dd`, `#61afef`, `#e06c75`; `Panel.qml:174-195` and throughout), so it does not follow the Omarchy theme for those elements.

Screenshots: the current `preview.png`, `1.png`, `prev.png` and `omatorrent.png` are byte-identical 1536×1024 images. They appear to be a **rendered marketing mockup rather than a capture**. They show two panels side by side under the banner "OmaTorrent for Omarchy Quattro", with category pills ("Software, Linux Distros, Documentation") that do not exist in Panel.qml and TPB/Lime/Nyaa provider badges the code no longer ships. An older history image (`git show 0e19f52:preview.png`, commit message "based on latest live UI screenshot", 1376×768) is closer to the real UI: a single bordered popout with the two tab pills, a search box, the seven category pills, provider and sort dropdowns, stacked result cards (badge, title, size, seeds, leechers, copy and send icons) and the footer hint "Click to send to qBittorrent · Click to copy magnet · Esc to close".

---

## 4. Feature matrix: qBittorrent desktop parity checklist

Legend: **Yes** = implemented in code; **Partial** = a subset or backend-only support; **Planned** = in the roadmap or an open issue; **No** = absent; **Excluded** = explicitly out of scope. "C" = Cuciz/omatorrent, "U" = ucmz851/omatorrent.

| # | qBittorrent feature (desktop) | Cuciz/omatorrent (Sprout) | ucmz851/omatorrent |
|---|---|---|---|
| **Transfer list** ||||
| 1 | Transfer list with configurable, sortable columns (name, size, progress, status, seeds, peers, speeds, ETA, ratio, category, tags, added/completed on, tracker, save path...) | Partial: name, state, progress track, ↓/↑ (when nonzero) or ETA and ratio; fixed name sort; no column choice (`Panel.qml:459-470`, delegate `:1022-1130`) | Partial: cards with state, %, ratio, name, bar, ↓/↑, done/size, ETA, DL-limit badge; at most 100 torrents (`Panel.qml:1455-1640`, `torrent_engine.py` `MAX_QBIT_TORRENTS`) |
| 2 | Status filter sidebar (All, Downloading, Seeding, Completed, Running, Stopped, Active, Inactive, Stalled, Checking, Moving, Errored) | Partial: All, Active, Downloading, Seeding, Paused, Completed (`Panel.qml:92, :429`) | No (transfers are not filterable) |
| 3 | Category filter | No (`category` is carried in the normalized item but not filterable) | No |
| 4 | Tag filter | No (tag *assignment* is Planned 0.6, #14) | No |
| 5 | Tracker filter | No | No |
| 6 | Name search within the transfer list | No | No (the search box searches indexers, not transfers) |
| 7 | Multi-select / bulk actions | Excluded: one hash per mutation, never `all` (docs/QBITTORRENT.md design rules) | No |
| **Adding torrents** ||||
| 8 | Add magnet/URL | Yes: magnets only, validated by the daemon (`torrent.add`, `client.go:462`) | Yes from search results (`launchTorrent`, `Panel.qml:162`); no paste field |
| 9 | Add .torrent file | Excluded before 1.0 ("non-goal for 0.3", docs/QBITTORRENT.md) | No |
| 10 | Add-dialog options (save path, category, tags, start stopped, skip check, content layout, rename, file selection, stop condition, queue top) | Excluded: every option is listed "NOT NEEDED BEFORE 1.0" (docs/QBITTORRENT.md, Add magnet) | Partial: the engine accepts `savepath` for `add` (`torrent_engine.py:798-802`), but the UI never passes it |
| **Per-torrent actions** ||||
| 11 | Pause/resume (stop/start) | Yes, with a version-gated fallback (`client.go:427-445`) | Yes, trying 5.x then 4.x on 404/400 (`torrent_engine.py:789-794`) |
| 12 | Force start | Planned 0.6 (#14) | Yes (`setForceStart`, `Panel.qml:1815`) |
| 13 | Delete with/without files plus confirmation | Yes: two explicit, separately confirmed paths (`Panel.qml:1197-1250`) | Partial: both exist but run **immediately, without confirmation** (`Panel.qml:1570, :1867`; the reviewer blocked this in #3231) |
| 14 | Force recheck | Planned 0.6 (#14) | Yes (`recheck`, `Panel.qml:1840`) |
| 15 | Force reannounce | Planned 0.6 ("possibly", ROADMAP; #14) | No |
| 16 | Set location / move | No | Partial: `set_torrent_location` exists in the engine (`torrent_engine.py:821`) but is not wired to the UI |
| 17 | Rename torrent | Excluded before 1.0 (capability matrix) | No |
| 18 | Sequential download / first-last piece priority | Excluded before 1.0 (add options list) | No |
| 19 | Super seeding | No | No |
| 20 | Per-torrent speed limits | Planned 0.6 (#14) | Yes: preset chips (`Panel.qml:1730, :1779`) |
| 21 | Per-torrent share ratio / seeding time limits | No: researched (`setShareLimits` in the capability matrix) but not scheduled | No |
| 22 | Queue position (top/up/down/bottom) | Planned 0.6 (#14) | No |
| 23 | Set category / tags | Planned 0.6 (#14) | No |
| 24 | Copy magnet / hash / name | No | Partial: copy magnet for *search results* only (`wl-copy`, `Panel.qml:155`) |
| 25 | Open containing folder / preview | No | Yes: open folder through `xdg-open` (`Panel.qml:1228, :1683`) |
| 26 | Automatic Torrent Management toggle | No | No |
| **Details tabs** ||||
| 27 | General tab (pieces bar, availability, hash, comment, dates) | No | Partial: the drawer shows only the save path |
| 28 | Trackers tab (list, add, edit, remove) | No (capability matrix only) | No |
| 29 | Peers tab (list, add, ban) | Excluded before 1.0 ("NOT NEEDED") | No |
| 30 | HTTP sources (web seeds) | No | No |
| 31 | Content tab: file tree, per-file progress, file priorities | Planned 0.7 (ROADMAP) | No |
| 32 | Per-torrent speed graph | Planned 0.8 metrics/history (ROADMAP) | No |
| **Global** ||||
| 33 | Global download/upload limits | Planned 0.6 (ROADMAP says "speed limits"; #14 lists per-torrent only, so the global scope is unclear) | Yes: preset chips (`Panel.qml:1382, :1425`) |
| 34 | Alternative speed limits ("turtle") | No | Yes (`toggleSpeedLimitsMode`, `Panel.qml:1182`) |
| 35 | Alt-limit scheduler | No | No |
| 36 | Queueing settings (max active downloads/uploads) | No | No |
| 37 | Global share ratio limits and action | No | No |
| 38 | Category management (create/edit with save path) | No (only assignment is Planned 0.6) | No |
| 39 | Tag management | No (only assignment is Planned 0.6) | No |
| 40 | RSS reader | Excluded (#14 "Out of scope: … RSS") | No: RSS feeds are only a *search source* type |
| 41 | RSS auto-download rules | Excluded (#14) | No |
| 42 | Search with qBittorrent search plugins | Excluded (#14 "search") | Partial: a **separate** search aggregator (archive.org, RSS, Torznab, JSON API), not qBittorrent's plugin system (`torrent_engine.py:856-1144`) |
| 43 | Torrent creator | No | No |
| 44 | Statistics | Partial: current-state dashboard (counts, total, done and remaining sizes, free space); history Planned 0.8 | Partial: global speeds, active counts; DHT nodes are fetched but not shown |
| 45 | Execution log | No (0.8 "diagnostics" is vague) | No |
| 46 | IP filter / banned peers | No | No |
| 47 | Encryption / anonymous mode | No | No |
| 48 | Network interface binding (VPN kill-switch style) | Excluded: "VPN monitoring is removed from Sprout's scope entirely" (ROADMAP) | No |
| 49 | Preferences editing (incl. default save path) | No | Partial: default save path only (`setPreferences`, `Panel.qml:1303`) |
| 50 | Free disk space / connection status / DHT in status bar | Yes: free space and status classes in the dashboard and panel | Partial: connection status and version |
| 51 | Completion notifications | No | No |
| 52 | Remote WebUI connection (host, auth, HTTPS) | **Yes**: LAN, remote, reverse-proxy sub-path, HTTPS with system roots, CA or pin, Secret Service credentials (PR #10) | No: `127.0.0.1` only, no auth, localhost bypass required |
| 53 | Port auto-detection | No (the URL is entered in settings) | Yes: reads `qBittorrent.conf` and probes common ports (`torrent_engine.py:555-590`) |

Rough score, counting Yes as 1 and Partial as 0.5 over the 53 rows: Sprout ≈ 6.5 (5 Yes, 3 Partial, and 9 more rows Planned for 0.6-0.8), OmaTorrent (ucmz851) ≈ 14 (9 Yes, 10 Partial). Neither is close to parity. Sprout is ahead on correctness and remote use. ucmz851 has more controls but each one is less safe.

---

## 5. Design language and look-and-feel

**What makes Sprout look native**:

1. It reuses first-party components directly instead of imitating them: `BarWidget`/`WidgetButton`, `Ui.Panel`, `KeyboardPanel`, `PanelKeyCatcher`, `BorderSurface`, `PanelSectionHeader`, `PanelSeparator`, `Border.surfaceSpec`, `Util.alpha`. It also follows the patterns of existing built-ins: the `clock` popout for the panel, `emojis`/`clipboard` for the overlay, and `menu` for routing between bar widget and overlay. docs/QUICKSHELL.md lists each reference file under `/usr/share/omarchy/shell/`.
2. Its visual rule is written down: "indistinguishable from a first-party Omarchy plugin — native tokens, transparent-first surfaces, native spacing/radii, minimal chrome, no RGB/cyberpunk decoration, no standalone-desktop-app look, no card-dashboard aesthetic" (docs/QUICKSHELL.md). The brand rule is "95% Omarchy / 5% Sprout" (docs/BRAND.md).
3. Controls are **text glyphs, not buttons**: ⚙ ▤ + ▶ ‖ ✕, "Add", "Cancel". It uses accent colour for actions, `urgent` for anything destructive or failing, and `muted` for the progress track background. Row actions appear on hover only, which keeps the list dense.
4. The brand is a **Canvas-drawn pixel glyph** whose geometry is embedded as grids in QML (`SproutGlyph.qml`). It is painted in the theme foreground, stays legible at bar-font size, and is mono-capable. The colour animation respects `bar.foregroundAnimationEnabled`. A Python generator makes the SVG and PNG assets from the same grids, and `tools/validate_brand.py` checks that the copies stay in sync.
5. **Degraded states are part of the design.** Each failure class has its own wording ("The connection was NOT downgraded.") and offers one path to fix it. Last-known data is dimmed and labelled. Fields that are unknown are left out rather than shown as zero.
6. A large view gets its own overlay plugin and is destroyed on close, so the bar and popout stay compact.

**Worth copying from ucmz851**:

- The search field gets focus as soon as the panel opens.
- Escape steps back one layer at a time: dropdown, then search text, then panel.
- Middle-click on the bar icon refreshes.
- Preset chips for speed limits.
- One-click "copy magnet" (`wl-copy`) and "open folder" (`xdg-open`).
- Explicit "Auto-Detect" and "Connect" buttons in the disconnected state.

**To avoid**:

- ucmz851's hard-coded hex palette, which breaks Omarchy theme switching.
- Its card-heavy, bordered-tile look (bordered chips and boxes everywhere), which Sprout's rules call the "card-dashboard aesthetic".
- Forking `python3` every 2.5 s from inside the shell.
- Destructive actions on a single click.
- Marketing images that do not match the real UI.
- From Sprout: right-aligned dashboard values clipped against the card edge, a state word that is invisible in rows, and the mouse-only list.

---

## 6. Gotchas and lessons from their code, issues and PRs

1. **An empty or degraded message that never rendered** (Cuciz #12, PHASE051.md, PR #13). The empty-state and degraded texts were children of a `ListView` whose height was bound to `min(contentHeight, 340)`. With zero rows the view collapsed to 0 px and the messages could not appear. They had not rendered since Phase 0.2, which went unnoticed because the dev backend always had torrents. The fix moved the list into a fixed area with the messages as overlays. That caused the opposite regression: a permanently 340-unit tall panel. The final fix sizes to content while rows exist and uses a 100-unit floor only when the count is 0. In the same issue: `resetSession()` did not clear `applyingSnapshot`, so a disconnect in the middle of a snapshot made later deltas skip view rebuilds. The issue also notes that the daemon's 30 s idle read deadline silently drops a client that only listens.
2. **Concurrency ordering that passed tests vacuously** (Cuciz PR #10, PHASE05.md). The commit message and PR body claimed that `configure` took its rollback snapshots *after* acquiring exclusivity. The production code still did it before, because the reorder edit had been lost. The accompanying tests passed either way. With two concurrent configures, one could roll back over the other's committed state. An external review caught it, and the fix added a deterministic gating regression test (`TestDeterministicStaleSnapshotConcurrency`) instead of relying on sleeps.
3. **`secret-tool store` exits 2 under a minimal environment** (PHASE05.md, commit 1cfd2cb). It failed with "Invalid byte sequence in conversion input", a GLib charset conversion of the item label in the daemon's stripped child environment. The fix uses an ASCII label and passes the locale variables through.
4. **Omarchy shell plugin traps** (docs/QUICKSHELL.md, PHASE04.md):
   - Adding an `overlay` kind to a bar-widget plugin silently reroutes `omarchy-shell shell toggle <id>`.
   - Calling `shell.hide()` from inside `close()` recurses with a RangeError; user-initiated closes must go through a separate `dismiss()`.
   - Overlay and panel plugins must be listed in `~/.config/omarchy/shell.json` `plugins` to be summonable.
   - `WlrLayershell` requires `import Quickshell.Wayland`.
   - After a hot plugin rescan, the first summon can run a **stale component**, so restart with `omarchy-restart-shell` before debugging the QML.
   - The Quickshell `Socket.write()` adds no newline.
   - An initial `connected: true` never fires `connectionStateChanged`, so the hello has to be bootstrap-aware.
5. **qBittorrent WebAPI facts the hard way** (docs/QBITTORRENT.md; live on 5.2.3 / WebAPI 2.15.1):
   - `torrents/pause` and `resume` return **404 on 5.x**.
   - `stop`, `start` and `delete` return 200 with an empty body even for unknown hashes, so the only real confirmation is synced state.
   - `add` returns **409 for both duplicates and malformed magnets**. On 5.2 it returns JSON (`added_torrent_ids`), plus 202 when pending.
   - `rid` is **scoped to the session**: without a cookie jar every poll is a full update, even under the localhost bypass.
   - Login on 5.2 returns 204 or 401; on 4.6-5.1 it returns 200 with "Ok." or "Fails.". 401 on a normal call means an Origin/Host check failed, and 403 means no session or a ban.
   - Five failed logins ban the IP for 1 h.
   - `free_space_on_disk` describes only the *default* save path.
6. **"-1 B" free space**. In the remote light-theme dashboard capture, the dashboard shows `Free space (default save path) -1 B`. The daemon passes `free_space_on_disk` through untouched (`syncer.go:480-482`) and QML formats any number (`Dashboard.qml formatBytes`). A negative sentinel from the backend should be treated as *unknown*. It is a small bug, but it breaks the project's own "unknown ≠ zero" rule.
7. **Seven review rounds to a closed ticket** (ucmz851; marketplace #2005, #3231, reviewer HANCORE-linux):
   - (a) Unbounded `urllib` reads plus a QML `StdioCollector` that collects everything before checking `text.length` could run the **shell process** out of memory.
   - (b) Bounding each response was not enough: the *aggregated* stdout also needed a ceiling.
   - (c) The fixed list of compiled-in indexers was "composed of sites whose primary purpose is indexing infringing copies". The reviewer called that a listing-policy question and made it the blocker; moving to user-configured indexers changed "the character of the question".
   - (d) The credentials file went from `open("w")`, to 0600, to `O_NOFOLLOW`, to a held directory-FD chain with `renameat` and directory fsync, and then still had to fail closed without unlinking.
   - (e) The Torznab `apikey` in the URL combined with followed redirects could leak the key.
   - (f) Delete-with-files on one click, no process teardown, and no cap on the number of indexers.

   The ticket closed for inactivity. Earlier, #2005 had been listed and was then delisted by a maintainer (PR #3028). The delisting retired the ID, which is why the resubmission needed a new one.
8. **Marketing surfaces are reviewed too** (ucmz851 #1). The README hero image `preview.png` is also the marketplace card. Another user asked for its risky demo search to be replaced with Linux ISOs. The replacement is a rendered mockup. At HEAD, the empty-state copy still says "search across ThePirateBay, LimeTorrents, YTS, EZTV, FitGirl, and Nyaa" (`Panel.qml:974`), which contradicts the commit that removed those endpoints.
9. **Polling only while open makes the bar indicator stale** (ucmz851). `qbAutoTimer` runs only when `opened` (`Panel.qml:205-211`), but `BarWidget.getStatusColor` reads the panel's last poll, so the bar icon colour and tooltip show old data while the panel is closed. Sprout's bar widget instead has its own 2 s status client.

---

## 7. Sources

**Repositories (read at the exact commits above):**

- Cuciz/omatorrent@e35b03f:
  - Docs: `README.md`; `docs/{PRODUCT,ROADMAP,ARCHITECTURE,IPC,QBITTORRENT,QUICKSHELL,SECURITY,BRAND,TESTING}.md`; `docs/adr/0001-0009`; `docs/agent/PHASE0{,2,3,4,5,51}.md`.
  - Plugins: `plugins/local.omatorrent/{manifest.json,BarWidget.qml,Panel.qml,MutationClient.js,SproutGlyph.qml}` and `plugins/local.omatorrent-dashboard/{manifest.json,Dashboard.qml}`.
  - Daemon: `omatorrent-service/internal/{qbittorrent/client.go,state/syncer.go,state/dashboard.go,ipc/protocol.go,ipc/server.go,secrets/secrets.go,connection/*}` and `cmd/omatorrent-service/main.go`.
  - Packaging and tests: `packaging/systemd/omatorrent-service.service`, `tools/test_quickshell.sh`.
  - Screenshots: `docs/screenshots/phase051-*.png`, `phase05-panel-tls-untrusted.png`, `phase03-panel-live.png`.
- ucmz851/omatorrent@28b9671: `README.md`, `manifest.json`, `BarWidget.qml`, `Panel.qml`, `scripts/torrent_engine.py`, `preview.png` (= `1.png` = `prev.png` = `omatorrent.png`), and the history image `0e19f52:preview.png`.

**GitHub:**

- Cuciz PRs #2, #4, #6, #8, #10, #13 (all merged). Issues #1, #3, #5, #7, #9, #11 (closed) and #12, #14 (open).
- ucmz851 issue #1.
- Omarchy marketplace issues: https://github.com/omacom/omarchy-plugin-marketplace/issues/2005 and /issues/3231. Context listings: #364 (OmaqBT), #3864 and #9107 (widget.torrents), #6538 (Torrents), #3094 (omARR), #9451 (Omarchy Proton).
- Marketplace delisting: https://github.com/omacom/omarchy-plugin-marketplace/pull/3028. Listing URL at publication: https://omarchyplugins.com/plugin.html?id=ucmz851.omatorrent (that domain now redirects to https://plugins.omarchy.org/; live data is at /catalog.json).
- Web-search hits: https://github.com/Aweiward/omaqbt, https://omaseed.app / https://github.com/diogochaves/omaseed, https://github.com/Xfedis/omarchy-torrents, https://github.com/andershfranzen/omarchy-torrents, omacom/omarchy issue #14672 (as reported by the web agent), and https://github.com/qbittorrent/qBittorrent/wiki/List-of-known-alternate-WebUIs

**External references cited by the repos:**

- Omarchy Quattro shell-plugins manual: https://github.com/basecamp/omarchy/blob/quattro/manual/32-shell-plugins.md
- qBittorrent WebUI API wiki: https://github.com/qbittorrent/qBittorrent/wiki/WebUI-API-(qBittorrent-4.1)

**Local evidence copies:** `/tmp/claude-1000/-home-oliver-Projects-odcpw-fbittorrent/d7aee677-4c3c-40be-a662-cab58b5a412d/scratchpad/research/{cuciz,ucmz851}/repo`, `.../research/images/`, `.../research/gh/*.json`.

---

## 8. Implications for fbittorrent

This section is based only on the two OmaTorrent projects. fbittorrent's own source was not read; the repo's file names (`src/ipc.rs`, `plugin/oliver.torrents/`, `src/search.rs`, `src/rss.rs`, `src/qbt.rs`) are known only from `git status`. Treat each item as "check whether fbittorrent already does this".

1. **Write the shell IPC contract down and test against fixture files.**
   *Evidence:* Sprout's `contracts/ipc/v1/*.txt` example frames are loaded by its Go tests, "so examples cannot silently drift" (docs/IPC.md). The protocol uses an integer major version in `hello`, adds only new message types, sets exact key sets and a 4096-byte frame cap, and never echoes the payload in error frames.
   *For fbittorrent:* put a `docs/IPC.md` and fixture frames next to `src/ipc.rs`, and have both Rust tests and a QML-side check read the same files.
2. **Push snapshot-then-delta to the widget instead of re-sending full lists.**
   *Evidence:* v1.1 sends `snapshot.begin/item/end`, then `delta{seq,changed,removed}`, with backpressure on the snapshot and a 256-frame bounded queue that disconnects slow clients, which then rebuild. On the QML side (`Panel.qml:370-480`), snapshot items only fill a map, the order is sorted once at `snapshot.end`, data-only changes call `ListModel.set` in place, and only membership or order changes rebuild.
   *For fbittorrent:* librqbit holds the state in-process, so producing the deltas is cheap, and the QML half can be copied almost line for line.
3. **Treat a mutation as "accepted", then "confirmed by state", with idempotency refs.**
   *Evidence:* ADR-0006 and `MutationClient.js` use a client `ref` per attempt and a daemon replay ring of 64. While pending, the row shows "pausing…", which is cosmetic only. A timeout means "ambiguous", not "failed". The panel re-sends the *same* ref after 12 s, and a remove is never retried automatically.
   *For fbittorrent:* this prevents optimistic UI lies when engine calls return before state settles, and double deletes when the widget reconnects.
4. **Make destructive intent explicit at every layer.**
   *Evidence:* Sprout makes `delete_files` a required boolean (a missing value is a protocol violation), offers two visually distinct confirm paths, and sends one hash per destructive call, never `all`. The marketplace reviewer blocked ucmz851 partly for one-click "Delete with Files" (#3231).
   *For fbittorrent:* the CLI, TUI and widget should all require explicit "remove" versus "remove + delete data" choices, with confirmation in the GUI surfaces.
5. **Degrade truthfully: "unknown" is not zero.**
   *Evidence:* the v1.3 degraded response leaves out speeds, `free_space` and `active`, and sends only `last_known`, which the UI dims and labels. The status classes are distinct: daemon offline, backend unreachable, auth, TLS, secrets. Sprout's own "-1 B" free-space bug (gotcha 6.6) shows why negative or sentinel values must also become "unknown".
   *For fbittorrent:* use an `Option`/absent field in the IPC for every metric that can be unknown, and give the bar widget distinct OFFLINE (daemon) versus ERROR (engine) states like Sprout's `qBT OFFLINE` and `qBT ERROR`.
6. **Split the large view into a separate overlay plugin.**
   *Evidence:* Sprout ships `local.omatorrent` (`kinds:["bar-widget"]`) plus `local.omatorrent-dashboard` (`kinds:["overlay"]`, a full-anchor `PanelWindow` on `WlrLayer.Overlay`, exclusive keyboard focus, scrim plus a `BorderSurface` card, not `keepLoaded`). It is toggled with `omarchy-shell shell toggle <id>`, and a 50x open/close test left the daemon's fd count flat. Adding `overlay` to the bar-widget plugin breaks toggle routing, and `close()` and `dismiss()` must stay separate (QUICKSHELL.md).
   *For fbittorrent:* a "manager" overlay (shows, RSS, search) can stay at zero cost while hidden, and the bar popout can stay small.
7. **Harden the Quickshell socket client against the known pitfalls.**
   *Evidence:* `BarWidget.qml` sends `JSON.stringify(msg)+"\n"` because `write` adds no newline. It sends a bootstrap-aware hello, because an initial `connected:true` fires no signal. It keeps one request in flight with id matching, resets after a 6 s stuck response, and runs a self-healing reconnect timer with backoff from 1 s to 10 s that does not depend on socket signals. It guards `JSON.parse` and ignores lines longer than 4096 bytes.
   *For fbittorrent:* check `plugin/oliver.torrents/Service.qml` against this list.
8. **Theme only with Omarchy tokens and brand with a monochrome glyph.**
   *Evidence:* all Sprout colours come from `Color.*`, `Color.menu.*`, `Style.*`, `bar.barForeground` and `bar.urgent`, and dark and light themes were verified with screenshots. The brand is a Canvas grid glyph in the theme foreground, sized to the bar font, with animation gated on `bar.foregroundAnimationEnabled`. ucmz851's hard-coded hex colours (`#87c095` and others) are the counter-example.
   *For fbittorrent:* audit the widget for literal colours, and add a light-theme screenshot to the regression evidence.
9. **Add a smoke test that runs a separate Quickshell instance.**
   *Evidence:* `tools/test_quickshell.sh` writes a temporary `shell.qml` that imports the production `MutationClient.js`, runs `timeout 100 qs -p $DIR`, speaks the real IPC to the running daemon, and checks `OTQS-*` marker lines. Mutations target a **disposable magnet with a random infohash** that can never resolve, so the user's torrents are never touched.
   *For fbittorrent:* this complements the existing tmux TUI smoke test (`agent_ergonomics_audit/audit/regression_tests/tui_smoke.sh`) by covering the plugin-to-daemon path without disturbing the running `omarchy-shell`.
10. **Before going to the marketplace, follow the review bar that ucmz851 hit.**
    *Evidence:* #3231 shows what the reviewer checks:
    - no compiled-in indexers of infringing sites; the reviewer treats them as a listing-policy matter;
    - indexers configured by the user;
    - bounded network reads *and* bounded aggregate output into the shell;
    - 0600 credential files written atomically and failing closed;
    - HTTPS required when an API key is present, and no cross-origin redirects;
    - process teardown on destroy;
    - confirmation for deleting data;
    - marketing images that match the real UI (ucmz851 #1).

    *For fbittorrent:* a recent commit, "search: bundle the nyaa.si engine", makes item (a) directly relevant. Decide before submitting whether search sources ship as defaults or stay user-configured. Torznab/Jackett/Prowlarr support, as ucmz851 did it, is a reviewer-friendly way to keep search.

Two more ideas, each backed by one source:

- If fbittorrent keeps a qBittorrent backend (`src/qbt.rs`), take docs/QBITTORRENT.md's verified facts as acceptance tests: 404 for `pause` on 5.x, 409 for add of a duplicate or a malformed magnet, `rid` that needs a cookie jar, 204/401 login on 5.2, and a sticky auth failure after 3 attempts to stay below the 5-attempt ban.
- For credentials, Sprout's `secret-tool` pattern needs a stdin secret, an ASCII label and passed-through locale variables (gotcha 6.3).
