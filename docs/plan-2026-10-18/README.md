# Plan 2026-10-18: fbittorrent look, feel and features

- [PLAN.md](PLAN.md): the implementation plan (round 6: the owner's answers folded in: one stop rule, "the torrent
  icon in the bar is lit while fbittorrent runs; click its power button and it stops, and is done", the Asupersync
  fork accepted with an owner checkpoint; round 4 integrated a fresh full read and round 5 verified references and
  numbers by script): decision, daemon lifetime and API v1,
  Quickshell and TUI design specifications, feature roadmap, work breakdown with dependencies,
  tests and logging contract, verification scripts.
- [review-log.md](review-log.md): one entry per review round: what changed, what was added, what
  was corrected (including wrong citations), what was deliberately left or deferred; rounds 2 to 4 have a
  verdict per review item, the reconciliation notes, the self-containment transcripts (round 3; round 4's
  are in its review) and the DAG and mockup-width script outputs; round 5 lists every correction it made
  (before and after), the ID-resolution script output and the steady-state verdict; round 6 records the owner's
  answers verbatim, the stop rule as written and every location it changed.
- [reviews/](reviews/): the round-2 architecture and product/UX reviews, the round-3 consistency and
  validation reviews and the round-4 fresh read, as received (read-only inputs).
- [reality-check.md](reality-check.md) ([RC]): audit of the current code: what works, live defects, IPC contract, qBittorrent feature inventory, UX score, tests, risks.
- [frontend-options.md](frontend-options.md) ([FO]): Omarchy and Quickshell platform facts, Rust GUI toolkits, topologies, decision matrix, API sketch.
- [omatorrent-research.md](omatorrent-research.md) ([OR]): the two OmaTorrent projects, their architecture and UI, parity matrix, gotchas, marketplace review bar.
- [evidence/](evidence/): build and test logs, installed-binary contract dumps, lifecycle and autolaunch probes.
- [screens/](screens/): TUI text and ANSI captures at 160×45, 80×24 and 60×20, the bar widgets and a real Ghostty window (PNG).

Read PLAN.md first; it cites the other three documents as [RC], [FO] and [OR].
