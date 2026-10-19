//! Existing help behavior.
use super::*;

impl TuiApp {
    pub(super) fn help_text(&self) -> String {
        let lifetime = if self.cfg.native() { "(Q)uit and stop the engine · Shift+Q also quits" }
            else { "(Q)uit · Shift+Q quits and stops qBittorrent" };
        let add = if self.cfg.native() { "(A)dd URL/magnet · F4 toggles seeding in the add prompt" }
            else { "(A)dd URL/magnet · F2/F3/F4 sequential/first-last/upload" };
        let files = if self.cfg.native() { "FILES    (Space) Include/skip · (1) Include\n         (A)ll files · (N)one · (Esc/Enter) Close" }
            else { "FILES    (Space) Include/skip · (1/2/3) Normal/high/max\n         (A)ll files · (N)one · (Esc/Enter) Close" };
        let streaming = if self.cfg.native() { "Streaming priority is automatic; file selection supports include/skip." }
            else { "(S)equential · (F) First/last priority" };
        let rss_read = if self.cfg.native() { "(M)ark read/unread or Space: selected item only" }
            else { "(M)ark read or Space: selected item only (legacy engine)" };
        format!("\
GLOBAL   (1) Torrents · (2) Search · (3) Shows · (4) RSS
         (Tab/Shift+Tab) Next/previous screen
         (?) Help · (Esc) Leave input, close a dialog or return
         Letters in parentheses use lowercase; Shift is explicit.
         (F6) Download queue, encryption and VPN (native engine)
         {lifetime}
         {add}
         (C)lipboard · arrows or j/k move · g/Shift+G first/last

TORRENTS (Enter) Files · (O)pen downloaded file/folder
         (P)ause/resume · (U) Allow/stop seeding
         (D) Remove, keep files · (Shift+D) Delete with files
         (I) IMDb · (W)ikipedia · (Shift+F) Follow this show
         {streaming}

SEARCH   Query: type + (Enter) Search; empty query browses lists
         (Down/Esc) Leave query · (/) Edit query from results
         Results: (Enter) Download · (Shift+O) Source page
         (S) Providers · (B)rowse lists · (F)ollow show
         (I) IMDb · (W)ikipedia; press Esc before ? in the query

BROWSE   (Enter) Load selected list or open a browser-only site
         (Esc) Results · (/) Query

PROVIDERS (Space) Toggle · (A)ll · (N)one · (O)pen site
          (Esc/Enter) Close; choices are saved

{files}

RSS      (Left/Right) or h/l: focus Feeds/Items
         Feeds: (Enter) Items · (N)ew feed · (X) Remove feed
         Items: (Enter) Download
         {rss_read}
         (Shift+M) Mark all items in the selected feed read
         (R)efresh feeds · (Shift+R) Reload saved feeds
         (F)ollow show from item · (I) Look up title
         Selecting or hovering leaves read state unchanged.

SHOWS    Starts in All unread, across every followed show.
         (Left/Right) or h/l: focus Shows/Releases
         Shows: All unread or one show · (Enter) Releases
         (N) Follow title/IMDb id
         Releases: (Enter) Download episode or season pack
         Inbox: (M)ark read or Space removes the selected release
         (U)ndo restores the last inbox read action
         One show: (M)ark read/unread or Space toggles one release
         (Shift+M) Mark all releases of one show read
         Select a followed show first; unavailable in All unread.
         (R) Check all · (Shift+R) Check selected show
         (X) Unfollow · (Shift+O) EZTV · (I) IMDb · (W)ikipedia
         Counts show unread releases, including old releases.
         Download history and read state are stored separately.
         Cached releases survive source removal and years between runs.
         Checking finds releases; downloading is always your choice.

MOUSE    Hover previews names; keyboard target stays selected.
         Click selects and focuses · wheel scrolls the pointed pane.
         Double-click activates the clicked row.
         A feed/show activates its child pane; an item downloads.
         Files/providers: double-click toggles the selected row.
         Click tabs or query box · long selected names scroll.

RELEASES ● unread · ↓ downloading · … queued/waiting · ‖ paused
         ✓ complete · + added earlier; separate from read state.

Native feed checks and transfers stop when the owning app closes.
Defaults: seeding off; (U) allows it per torrent.")
    }

    pub(super) fn help_max_scroll(&self) -> u16 {
        let height = self.last_bounds.get().height.min(34).saturating_sub(5);
        (self.help_text().lines().count() as u16).saturating_sub(height)
    }

    pub(super) fn view_help(&self, frame: &mut Frame) {
        let th = &self.theme;
        let b = frame.bounds();
        let w = 84.min(b.width);
        let h = 34.min(b.height);
        let area = Rect::new(b.x + (b.width - w) / 2, b.y + (b.height - h) / 2, w, h);
        let blank = Text::from_lines((0..area.height).map(|_| Line::styled(" ".repeat(area.width as usize), Style::new().bg(th.selection))));
        Paragraph::new(blank).wrap(WrapMode::None).render(area, frame);
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(th.border_focused()).title(" Keys ").padding(Sides::all(1))
            .style(Style::new().bg(th.selection));
        let inner = block.inner(area);
        block.render(area, frame);
        let rows = Flex::vertical().constraints([Constraint::Fill, Constraint::Fixed(1)]).split(inner);
        let text = self.help_text();
        let scroll = self.help_scroll.min(self.help_max_scroll());
        Paragraph::new(text).style(Style::new().fg(th.fg).bg(th.selection)).scroll((scroll, 0)).wrap(WrapMode::None).render(rows[0], frame);
        Paragraph::new("j/k ↑/↓ wheel scroll · PgUp/PgDn · Esc or click close")
            .style(Style::new().fg(th.accent).bg(th.selection)).wrap(WrapMode::None).render(rows[1], frame);
    }
}

