# fbittorrent TUI interaction review

Reviewed 2026-10-10. This review covers Torrents, Search, RSS and Shows, including
their input, file and provider views. It uses the current Rust handlers and
rendering code, the owned design library recovered through CASS, and primary
documentation for Zellij and Midnight Commander.

The combined implementation was installed after the final CLI/TUI fixture
passed actual terminal mouse clicks, selection at a nonzero RSS scroll offset,
hover preserving the keyboard target, visible read/unread updates, batch read
scope, refresh, F6 settings and close/reopen. Unit checks cover every tab label
cell at 80/120 columns and the two-row footer at 80×24. See
[the integration receipt](validation/2026-10-10/native-integration.txt) and
[the validation report](validation/2026-10-10/README.md). The broader acceptance
tasks below also describe future checks; they are not all completed by this
fixture.

The desired experience is a small, understandable torrent app: find a release,
inspect it, deliberately download it, and later return to the same feed or show
without losing track of what was reviewed. A coherent target, action and result
matter more here than adding more chrome or animation.

## Evidence and limits

The reported RSS mouse failure is a user observation. The review found concrete
source inconsistencies; it does not establish the cause of every failure in the
running terminal. Rendering and handlers are in `src/tui/mod.rs`; persistent read
state is in `src/rss.rs` and `src/shows.rs`.

Useful existing behavior includes four direct screen keys, visible focused pane
borders, single selection, double-click activation, scroll-offset-aware row hit
testing, delayed scrolling of long selected names, preserved selection identity
during RSS reloads, streamed search results, persistent read identifiers, and
separate show read/download history. Preserve those useful capabilities while
making their consequences legible.

Changes described as implemented below were made during this review turn. The
review subagent checked Rust syntax with Rust 1.98 `rustfmt --emit stdout` and
checked whitespace with `git diff --check`. Compilation, automated interaction
tests and real terminal checks belong to the integrating agent. A source check
does not prove the installed executable or terminal mouse protocol works.

## One interaction model for the four screens

| Surface | Selected object | Enter / double-click | Read action |
|---|---|---|---|
| Torrents | Torrent | Open its file selection | None; torrent state is transfer state |
| Search query | Query text | Run search; empty query opens provider lists | None |
| Search results | Release | Download selected release | None |
| Provider browse | List or browser-only source | Load list or open site; label describes which | None |
| RSS Feeds | Subscription | Focus its Items pane | Shift+M marks items in this feed read |
| RSS Items | Article | Download selected article | m/Space toggles selected item; Shift+M marks feed items read |
| Shows | Followed show | Focus its Releases pane | Shift+M marks releases of this show read |
| Shows Releases | Episode or season pack | Download selected release | m/Space toggles selected release; Shift+M marks show releases read |
| Files / Providers | Checkbox row | Double-click toggles row; Enter closes current dialog | Space toggles row |

The primary action varies by object, and the visible label must name it. “Enter
activate” is too vague to distinguish inspecting files from starting a download.
All-read always applies to the selected parent, never every feed or every show.
Lowercase m must never silently broaden from one release to an entire show just
because focus moved left.

There are two navigation levels. In the current app, 1–4 and Tab/Shift+Tab switch
screens, while Left/Right or h/l move between RSS/Shows panes. Keep that rule
explicit in Help and show the focused pane in the footer. Moving Tab to local
pane focus would be a deliberate interaction change, not a cosmetic polish;
Midnight Commander uses it that way, while fbittorrent already advertises Tab as
screen navigation. Decide that change as a whole, including Search input and
dialogs, if ordinary use still shows a need for it. [Midnight Commander manual](https://source.midnight-commander.org/man/mc.html#Directory_Panels)

## Prioritized findings

### Blocking: RSS pointer behavior needs actual event proof

`view_tabs()` computes visible tab rectangles and `on_mouse()` consumes them.
The rendered `Tabs` widget adds brackets to its labels; a hit rectangle based
only on label text shifts later tab targets. This is a concrete geometry defect,
and the integrating agent corrected that width and header overlap in this turn.
Test clicks across the whole RSS tab, including its label and closing bracket.

RSS Feeds also lacked the same Enter-to-child-pane behavior as Shows. Enter now
opens Items. A click selects; a double-click activates the selected row. For a
feed, activation enters Items; for an item, activation downloads. Verify all
three events with a scrolled feed and item list. Source-level behavior alone is
insufficient evidence for the user's running terminal.

### Important: scope, case and label were inconsistent

Earlier `on_shows_key()` made m mark every release when the parent pane had
focus, while the same key toggled one release in the child pane. RSS m marked
the entire feed even with an item selected. `view_status()` called that “mark
read,” concealing the broad scope.

The integrating agent changed m/Space to operate only on a selected child and
Shift+M to operate on the selected parent's children. Parent-pane m now explains
how to open the child pane or invoke the batch action. Native RSS can toggle
read/unread durably; legacy RSS supports marking read. In one show's full
history, the selection stays after a toggle, so pressing m again reverses it.

The default Shows view is now All unread, combining followed shows with a
visible unread count in the tab. In this inbox, m removes the selected release
and u restores that exact last release. Shift+M applies only after selecting
one followed show. Hover leaves the keyboard selection unchanged. Ordinary
pointer motion is explicitly enabled after terminal setup, and double-click
activation compares item identities so a refreshed row cannot activate an old
click target.

The footer now uses familiar mnemonic labels such as `(O)pen`, `(A)dd` and
`(M)ark read`. A read row offers `(M)ark unread`. Shift is spelled out for batch
and other distinct actions: `(Shift+M) All read`, `(Shift+D) Delete files`,
`(Shift+O) Source`, `(Shift+R) Check show`. The capital letter in an ordinary
mnemonic is typography; Help states that the action uses the lowercase key.
This avoids turning the requested presentation into a new ambiguous case rule.

### Important: hover and keyboard targets need distinct roles

Earlier `on_mouse()` called `focus_mouse_list()` before filtering the event kind,
then changed selection on `Moved`. Moving the pointer across the display could change
both the highlighted row and the scope of the next keyboard command. This is
particularly noticeable after using h/l to choose a pane while the pointer rests
over the other pane.

Implemented by the integrating agent in this turn: click establishes selected
row and keyboard focus, while hover records a preview target without selecting
or focusing it. Hover can reveal a long name after the existing delay. Wheel acts
on the pane under the pointer. Keyboard actions retain the selected row.
Double-click invokes the same primary action as Enter after selecting the same
object. Verify this with the stationary-pointer task below. Midnight Commander's
documented single-click selection and double-click execution provide a useful
precedent for that distinction. [Midnight Commander mouse support](https://source.midnight-commander.org/man/mc.html#Mouse_Support)

Use rectangles recorded during rendering for pointer targets. Currently most
`mouse_list_at()` rectangles reconstruct the layout separately from `view_*()`;
sharing actual rectangles will prevent later responsive-layout edits from
creating another divergence. This is also the FrankenTUI skill's layout-cache
pattern. Keep row offset and header/warning lines in the recorded row area.

### Important: “new,” unread, added before and complete are different states

`Show::is_new()` means absent from the `seen` set. A first follow can discover
years of old releases, and toggling a downloaded release unread makes it count
again. The integrating agent changed the user-facing Shows count to **unread**. A number by itself
or “new” suggests recently published or not yet downloaded.

The previous episode renderer chose one leading symbol from both read and
transfer state. An active or completed torrent took precedence over the unread
symbol. A previously grabbed release with no current torrent showed a down
arrow, which could imply an active download.

Implemented by the integrating agent: independent `●` unread gutter and transfer
symbol: `↓` downloading, `…` queued/waiting, `‖` paused, `✓` complete and `+` added
earlier. Help now explains this symbol vocabulary. Read means “reviewed this feed release”; it
does not mean watched, downloaded or present on disk. A successful add may mark
read, but a failed add must leave the marker and count unchanged. A season pack
must not silently erase per-episode read state.

### Important: the action legend competed with feedback

The former footer appended every shortcut and placed a potentially long status
message at the right. The `ftui::StatusLine` implementation clips left items to
the available space, so Help at the end was among the first useful controls
lost. The application also measured status width by character count rather than
terminal cell width.

Implemented: two constant footer rows. The first has focused-pane context at the
left, complete action labels packed in priority order, and Help/Close reserved
at the right. The second shows full-width feedback; an idle view uses it for
navigation hints. The one-row cost at 80×24 buys stable commands: transient
success or error text no longer hides m or Shift+M. Rendering and mouse layout
must use the same two-row reservation. Width uses the existing Unicode
display-width helper. Help is
accurate for current mode: while editing Query, `?` is text, so the footer says
`(Esc, ?) Help`; dialogs show their available Escape action. Empty-state guidance
now uses the same labels, and Search no longer claims a fixed 17 providers.

Recommended next step: a shared action definition should eventually generate
the footer, contextual help and clickable action targets. The same definition
should supply key, modifier, scope, enabled state and state-dependent label.
Changing words alone will not prevent future drift between help and handlers.
Zellij's own release notes identify hard-coded shortcut hints as a cause of
confusion when they diverged from actual bindings. [Zellij status-bar bindings](https://zellij.dev/news/sixel-search-statusbar/)

### Useful polish: responsive content and navigation completeness

Torrents/Search hide the details pane below 100 columns. RSS/Shows keep two
percentage-based panes, and Shows reduces metadata with width. At 80 columns
the release title gets little space; at smaller sizes two side-by-side panes
cease to carry a useful task.

Recommended: prioritize release key, unread state and recognizable title; drop
date, then size and optional rating before reducing the title further. At a
small terminal width, render the focused list full width with a parent
breadcrumb and Left/Right navigation. Keep the selected name accessible through
the existing delayed scroll and a full-title detail view. Preserve selection by
stable object identity during refresh and resize. Do not enlarge panels merely
to keep decorative poster space.

Arrow keys should be first-class discoverable navigation; j/k/h/l remain useful
expert aliases. RSS currently lacks Home/End navigation that Shows, Search and
Torrents have. PageUp/PageDown should move a viewport on all lists. Add these as
one consistent list-navigation change. At 80×24, 120×30 and 160×44, check the
actual task and complete action labels, not only that one fixture name appears.

## Why these recommendations follow the design references

CASS recovered the shared library workflow from `3dprint-cli`; direct file
inspection confirmed the books currently exist. CASS supplies project history,
while the owned corpus supplies the source text. A library hit is a locator, not
proof of having read a book or tested this interface.

- Norman, *The Design of Everyday Things*, revised 2013 edition, Chapter 1
  `text/part0007.html` (feedback and conceptual models), Chapter 2
  `text/part0008.html` (action cycle), and Chapter 3 `text/part0009.html`
  (knowledge in the world): the interface should explain the possible action,
  its mapping and its outcome. Applied here: visible focus, explicit action
  scope, dynamic labels and useful success/error feedback. [Owned reading copy](/home/oliver/Projects/knowledge/3dprint/library/books/corpus/norman-everyday-things-2013/reading.md)
- Cooper et al., *About Face*, fourth edition, physical PDF p.50 (printed p.20):
  expose the user's task through the represented model. Applied here: “Download,”
  “Read” and “Unread,” with transfer history distinct from read state. [Owned passage](/home/oliver/Projects/knowledge/3dprint/library/books/corpus/cooper-about-face-4e/reading.md#pdf-page-50)
- Rosenfeld et al., *Information Architecture*, fourth edition, physical PDF
  pp.183–184: labels form a consistent system with predictable syntax and
  granularity. Applied here: a shared single-item/batch rule and one notation
  for modifiers across screens. [Owned passage](/home/oliver/Projects/knowledge/3dprint/library/books/corpus/rosenfeld-information-architecture-4e/reading.md#pdf-page-183)
- Johnson, *Designing with the Mind in Mind*, third edition, physical PDF
  pp.159–160 (printed pp.144–145), and NN/G's recognition discussion: make
  available actions recognizable at the point of use. Applied here: contextual
  legends and Help, rather than requiring the user to memorize a flat list.
  [Owned passage](/home/oliver/Projects/knowledge/3dprint/library/books/corpus/johnson-mind-in-mind-3e/reading.md#pdf-page-159), [NN/G recognition and recall](https://www.nngroup.com/articles/recognition-and-recall/)
- Zellij documents mnemonic shortcuts displayed on screen. Adopt that
  discoverability principle without adding its terminal-multiplexer mode system
  to this app. [Zellij FAQ](https://zellij.dev/faq/#do-i-need-to-learn-new-keyboard-shortcuts)

The user named Norton Commander as a familiarity reference. The operational
precedent used here is its modern, actively documented relative Midnight
Commander. This report does not attribute fbittorrent's exact parenthesized
label style to an uninspected Norton manual.

## Focused acceptance tasks for the integrating agent

1. Click every part of each screen tab. On RSS, click a feed, enter its items,
   select a scrolled item, and double-click it. Confirm the intended item is
   submitted once and only successful submission changes read state.
2. With two unread children, m changes only the selected child; pressing m again
   reverses that same child. Shift+M changes all children of the selected parent.
   Reload and verify persistence. Other parents' counts remain independently
   explainable.
3. Test a completed release marked unread, a running download, an absent
   previously added torrent and a season pack. The displayed read count and
   transfer state should each have an unambiguous explanation.
4. At 80×24, show the dynamic read/unread action, Shift+M, focused pane and Help;
   then introduce a long success/error message. Confirm labels remain complete
   and Help remains accessible. Repeat with wide Unicode titles and at smaller
   terminal sizes.
5. Start Search, use partial streamed results, receive a provider warning, edit
   the query, browse a browser-only provider and return. Focus and available
   actions should stay understandable through every state.
6. Test keyboard use with a stationary pointer over the other pane. Hover should
   preview that row while keyboard read/download actions retain the selected
   object. Clicking the preview row should visibly establish the new target.

The next changes with the highest practical benefit are shared action
definitions and responsive list/navigation behavior. A
command palette, draggable panes, theme cycling and decorative motion should
only be added when a concrete use task demonstrates their value.
