# TUI render baselines (P0-16)

Generated from the unsplit `src/tui/mod.rs` at base revision `dd65e2c` using
FrankenTUI 0.6.0. These capture existing behavior, including clipped labels;
they are not the later P4 redesign mockups.

The nine surfaces each have 80×24, 120×30 and 160×45 baselines. Text comes
directly from frame cells, resolving pooled graphemes and skipping continuation
cells of wide characters. Blank cells become spaces; right padding is trimmed
so the files pass `git diff --check`. Every row ends with a newline, including
blank rows. Color is intentionally omitted. Before trimming, each row must
occupy exactly the frame's display width.

The local `.gitattributes` allows blank final rows in these text fixtures;
otherwise `git diff --check` would reject the empty rows of a full frame.
Trailing spaces remain subject to the normal whitespace check.

`fixture.rs` constructs local in-memory data: three torrents, two shows with
two unread releases, one feed with three items and ten search results including
the captured mojibake name. It does not run model initialization, subscriptions,
services or provider calls. The theme name is fixed to `default`, VPN binding is
off and no poster graphics are placed. Each fresh fixture uses the current time
for torrent creation so its immediately rendered relative label is `just now`;
release dates are fixed. No marquee target is armed.

Compare normally:

```sh
cargo test --locked tui::golden -- --nocapture
```

Regenerate deliberately, then review all changed baseline text:

```sh
FBT_UPDATE_GOLDEN=1 cargo test --locked tui::golden -- --nocapture
git diff -- src/tui/tests/golden/
```

A mismatch names the golden file and prints a unified diff. Never set the
update variable in CI. The 60×20 meta-test renders all nine surfaces without
creating extra goldens. The Unicode check verifies wide and combining text.
