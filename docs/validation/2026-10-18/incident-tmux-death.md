# Incident: swarm tmux server and Codex agents lost, 2026-10-18 13:13

Read-only investigation, 15:40 CEST. All times are local (CEST = UTC+2). Journal window searched: 11:30-15:35.

**Verdict (high confidence):** systemd-oomd killed the whole cgroup `app.slice/app-com.anthropic.Claude-1846.scope`
at 13:13:15 because app.slice memory pressure stayed above 50% for more than 20 s. That one scope held the Claude desktop
app, the orchestrator Claude Code (pid 3277226) and the old `fbittorrent` tmux server (pid 3782421). The server had inherited
the cgroup from the Bash tool that ran `ntm spawn`. The Codex agents were in their own `tmux-spawn-*.scope` cgroups and were
not killed directly. They died when the server's SIGKILL hung up their ptys. There was no kernel OOM kill, no crash, no
coredump and no reboot, and nothing else was killed between 13:00 and 15:28.

## Timeline

| Time | Event | Source |
|---|---|---|
| 11:52:47-50 | Old server 3782421 starts panes 3782422 (cod_1), 3782431 (cod_2), 3782671 (cod_3), 3782786 (cod_4). tmux moves each pane, not itself, into a `tmux-spawn-*.scope` | `journalctl --user`: "Started tmux child pane N launched by process 3782421" |
| 12:23:05 | Reviewer pane 3874324 (%4) | same |
| 12:33, 12:48 | rchd: dead-client lease recovery for cod_1 fails ("refusing overwrite"). cod_4 later reports rch "fell back locally" (conflicting /dp mapping) | `journalctl --user -u rchd`; orchestrator transcript 10:54:06Z |
| 12:49-13:01 | Orchestrator starts 5 local verification builds from its Bash tool with `RCH_CARGO_WRAPPER_BYPASS=1`. The last, at 13:01:14, is the asupersync timer-wheel lib-test build (task bpsjyvg72), still running at the kill | `~/.claude/projects/...fbittorrent/d7aee677-....jsonl` |
| 12:52, 13:02 | journald: "Under memory pressure, flushing caches" | `journalctl -k` |
| 12:52:51 | 2.4 GB available, zram 29.2/30.9 GB, one rustc at 11.6 GB RSS, load 11.5 | transcript (`free`, `ps`) |
| 12:58:31 | Last ntm `prompt_send` before the gap (the next is 15:31:48) | `~/.config/ntm/analytics/events.jsonl` |
| 13:01:19 | Last orchestrator transcript entry before the gap (idle, waiting on background tasks) | transcript |
| 13:12:58 / 13:13:14 | Last writes of the 3 working Codex rollouts, each cut off mid-turn (the other 2 had been idle since 12:56/12:57) | `~/.codex/sessions/2026/10/18/rollout-*T11-5[23]-*`, `*T12-23-09*` |
| 13:13:15.481 | oomd: "Considered 36 cgroups for killing". Top candidate is the Claude scope: 10.1G in use, PSI avg10 26.97, pgscan +37,227 since the last check | `journalctl` (systemd-oomd[861]) |
| 13:13:15.534 | "Marked ...app-com.anthropic.Claude-1846.scope for killing due to memory pressure for .../app.slice being 65.27% > 50.00% for > 20s with reclaim activity" | same |
| 13:13:15.958 | "systemd-oomd killed 707 process(es) in this unit", result `oom-kill` (scope peak 17.8G RAM, 17.4G swap) | systemd[1260] |
| 13:13:17-18 | Desktop launcher scope empties. Pane scopes of cod_3 (67f52625, peak 15.1G) and the reviewer (649e8d63) empty | systemd[1260] |
| 13:16:49 / 13:17:45 / 13:18:03 | Pane scopes of cod_2 (ebed3536), cod_4 (9f289ece) and cod_1 (cdd8b0f0) empty. Test children the agents had started kept running after the hang-up (new tmux test sockets `fbt-native-3987972` at 13:16 and `fbt-fbt-p0-15-3hq-3990044` at 13:18) | journal; `ls -la /tmp/tmux-1000/` |
| 13:18-15:26 | No Claude desktop running. Only snapper timers in the journal | journal |
| 15:26:50 | User relaunches Claude (uwsm gtk-launch), creating `app-com.anthropic.Claude-4039612.scope` | uwsm_app-daemon, systemd[1260] |
| 15:28:15 | The resumed session reports 4 orphaned background tasks (bp64w1ftm, bfpysul11, bg386vt1j, bpsjyvg72) | transcript |
| 15:30:41 | New swarm starts under `fbt-swarm-1792330240.scope` (`systemd-run --user --scope`) | journal |

How pane scopes map to agents: scope end time minus its logged wall-clock duration gives its start time, which matches the
"Started tmux child pane" time to the millisecond (for example 13:13:18.092 - 1h20m28.107s = 11:52:49.985, cod_3).

## Cause

1. **Mechanism.** Omarchy marks app.slice as the only oomd kill domain: `/usr/lib/systemd/user/app.slice.d/10-oomd.conf`
   (`ManagedOOMMemoryPressure=kill`, `ManagedOOMSwap=kill`) and `/etc/systemd/oomd.conf.d/10-omarchy.conf` (50% for 20s).
   When the limit trips, oomd SIGKILLs an entire child cgroup of app.slice: the one reclaiming fastest (pgscan rate), then the
   largest. This kill was triggered by pressure, not by swap (swap was about 50% of 61 GB).
2. **Why the tmux server died with Claude.** A process stays in its parent's cgroup. ntm ran from the orchestrator's Bash
   tool, so the tmux server stayed in the Claude scope. tmux 3.8 moves only pane children into `tmux-spawn-*.scope`.
   The current server 4043219 shows the same thing: it remains in the scope it was launched in.
3. **Why the Claude scope ranked first.** It had 10.1G resident and the top reclaim rate, because the orchestrator was
   compiling locally inside it with rch bypassed. The same asupersync command, rerun at 15:29:53 in the new Claude scope,
   reached rustc 14.2 GB RSS and a scope of 16.7 GB with 1.0 GiB available, then finished by 15:39. Not proven: whether the
   11.6 GB rustc seen at 12:52 was in this scope or in cod_3's pane (peak 15.1G). The transcript's parent-chain check used
   `pgrep -f 'rustc --crate-name' | head -1`, which matches the checking shell itself, so it does not settle the question.

**Ruled out.** (a) Kernel OOM killer: no "Out of memory" or "Killed process" in `journalctl -k` today (`dmesg` is denied,
"Operation not permitted"). (b) Teardown by the desktop or Claude Code: `disclaimer --pgroup` only calls `setsid` (`strings`),
the tmux server daemonizes into its own session anyway, and the desktop did not exit on its own; oomd killed it. (c) Claude
Code crashing: no coredump (`coredumpctl list`); it was one of the 707 processes. (d) tmux or the codex app-server: no
coredump, and the app-server daemons are still alive.

## Does `systemd-run --user --scope` fix it?

- **(i) Claude Code or desktop restart: yes.** The tmux server 4043219 and `ntm internal-monitor` 4044525 are in
  `app.slice/fbt-swarm-1792330240.scope`, the server's ppid is 1260 (`systemd --user`), and it runs in its own session.
  Linger=yes. Neither a kill of the Claude scope nor process-group teardown reaches it.
- **(ii) Memory pressure: only partly.** The swarm scope (about 94 MB) is still a child of app.slice, so oomd can pick it,
  but with so little memory it is unlikely to rank first. Each agent's `tmux-spawn-*` pane scope is its own candidate: a pane
  running a local build becomes the top reclaimer, and oomd kills that agent along with its build. The orchestrator is still
  exposed: whatever it builds runs inside the Claude scope. That repeated at 15:30-15:38 (14.2 GB rustc in
  `app-com.anthropic.Claude-4039612.scope`). oomd only stayed quiet because app.slice PSI stayed low.
- **A transient service (`--unit`, not `--scope`) adds nothing against oomd:** it sits in the same slice and is ranked the
  same way. `OOMScoreAdjust` only steers the kernel OOM killer, which did not act. Every process here runs at
  oom_score_adj 200, and the user manager can raise it but not lower it without CAP_SYS_RESOURCE (sudo).

## Prevention, ranked

1. **Run every local build in its own capped scope, never in the Claude scope or an agent's pane scope** (orchestrator and
   agents alike; panes have `DBUS_SESSION_BUS_ADDRESS`, so this works inside them):
   `systemd-run --user --scope --quiet --unit=fbt-build-$(date +%s%N) -p MemoryHigh=10G -p MemoryMax=14G -p MemorySwapMax=4G -- choom -n 1000 -- cargo ...`
   MemoryMax turns a runaway rustc into an OOM kill confined to that cgroup. Its high reclaim rate makes the build scope
   oomd's first pick instead of the desktop or an agent. `choom -n 1000` makes it the kernel OOM killer's first victim.
   Wire this into the rch shim fallback path, or the existing `flock /tmp/fbittorrent-release.lock` rule, plus marching orders.
2. **Mark the swarm, the panes and the orchestrator `ManagedOOMPreference=avoid`.** In systemd 262 oomd honours this for
   pressure kills when the cgroup and the monitored ancestor have the same owner (here both `oliver`, checked with `stat`).
   It is not honoured for swap kills. Commands: spawn with `systemd-run --user --scope -p ManagedOOMPreference=avoid ...`;
   after every spawn or respawn run `for u in $(systemctl --user list-units --plain --no-legend 'tmux-spawn-*.scope'
   'app-com.anthropic.Claude-*.scope' | awk '{print $1}'); do systemctl --user set-property --runtime "$u"
   ManagedOOMPreference=avoid; done`. Verify with `getfattr -n user.oomd_avoid /sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice/<unit>`.
   Only use this together with step 1. Otherwise builds inside those scopes are shielded too, and oomd kills Chrome or ChatGPT.
3. **Lower the peak.** Drop `RCH_CARGO_WRAPPER_BYPASS=1` from orchestrator verification. Fix the rch /dp mapping conflict
   and cod_1's stuck lease (the 3rd failure was at 13:13:47) so builds go to the 3090 instead of falling back to the laptop.
   Extend the flock rule to asupersync and test builds and to the orchestrator. Use `CARGO_BUILD_JOBS=2` for local fallback.
   The four worktrees mean four separate target dirs and four cold builds.
4. **Gate builds on pressure.** Before a local build, require `some avg10` < 10 in
   `/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice/memory.pressure` and MemAvailable > 6 GB.
   Otherwise queue it.
5. Not recommended: loosening Omarchy's 50%/20s limit, or using MemoryLow (it needs sudo on `user@1000.service`).

## Casualties

- **Killed by oomd** (707 processes, 13:13:15): the Claude desktop (1846) and its helpers; orchestrator Claude Code 3277226,
  its MCP servers (build123d, freecad, blender, kicad-mcp-pro, mcp-raker, 3d-printer) and its 4 background tasks (3 pane
  watchers plus the asupersync test); the old tmux server 3782421. Probably also a second, 7-day-old `claude` process that
  was listed at 12:52.
- **Lost through the pty hang-up:** all 5 Codex agents (cod_1-cod_4 and the reviewer). Uncommitted edits survived in the
  cod_1, cod_2 and cod_4 worktrees (orchestrator check at 15:29).
- **Unaffected:** rchd 790840 (`rchd.service`, logging without a gap); codex app-server daemons 926819/3098250 (ghostty
  scope, running since Oct 12/16); Chrome, ChatGPT, Hyprland (session.slice) and omarchy-shell. `systemctl --user --failed`
  lists only the dead Claude scope.
- **Not part of this incident:** the defunct codex 937768 has been an unreaped zombie of daemon 926819 since Oct 12 20:11.
  The system-level failed TPM units are unrelated as well.
