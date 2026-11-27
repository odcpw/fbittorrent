//! Responsive search list and poster card over the confirmed mirror.
use super::*;

impl TuiApp {
    pub(super) fn view_search(&self, area: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let extended = layout::Tier::at(area.width).extended();
        let rows = Flex::vertical()
            .constraints([Constraint::Fixed(3), Constraint::Fill])
            .split(area);
        let title = if self.search_running {
            format!(" Search {} ", SPINNER[self.tick % 4])
        } else {
            " Search ".to_string()
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(if self.search_focus_input {
                th.border_focused()
            } else {
                th.border()
            })
            .title(&title)
            .padding(Sides::horizontal(1));
        self.rects.borrow_mut().query = Some(rows[0]);
        let inner = block.inner(rows[0]);
        block.render(rows[0], frame);
        self.search_input.render(inner, frame);

        let cols = columns_with_details(rows[1], 140);
        if self.search_browse {
            self.view_browse(cols[0], cols[1], frame);
            return;
        }
        let enabled = self.plugins.iter().filter(|p| p.enabled).count();
        let title = if self.plugins.is_empty() {
            format!(" Results ({}) ", self.search_results.len())
        } else {
            format!(
                " Results ({}) · {}/{} providers ",
                self.search_results.len(),
                enabled,
                self.plugins.len()
            )
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(if self.search_focus_input {
                th.border()
            } else {
                th.border_focused()
            })
            .padding(Sides::all(0))
            .title(&title);
        let inner = block.inner(cols[0]);
        block.render(cols[0], frame);
        if self.search_results.is_empty() {
            let msg = if self.search_running {
                "Searching… Results appear as providers reply."
            } else {
                "Type a query, then (Enter) Search.\nEmpty query + Enter browses provider lists.\n(/) Edit query from results · (B)rowse lists"
            };
            Paragraph::new(msg)
                .style(th.dimmed())
                .render(pad(inner), frame);
        } else {
            let rows = Flex::vertical()
                .constraints([Constraint::Fixed(1), Constraint::Fill])
                .split(inner);
            let hdr = Rect::new(rows[0].x + 2, rows[0].y, rows[0].width.saturating_sub(2), 1);
            let width = inner.width.saturating_sub(2);
            let name = usize::from(width).saturating_sub(search_tail_width(extended)) as u16;
            let columns = [
                (Column::Name, name),
                (Column::Size, 9),
                (Column::Seeds, 6),
                (Column::Rating, 5),
                (Column::Uploaded, 10),
                (Column::Provider, 14),
                (Column::Peers, 6),
            ];
            let mut x = hdr.x;
            for (column, width) in columns.into_iter().take(if extended { 7 } else { 4 }) {
                let active = column.search_field() == Some(self.search_sort);
                let label = format!(
                    "{}{}",
                    column.label(),
                    if active {
                        if self.search_descending {
                            "↓"
                        } else {
                            "↑"
                        }
                    } else {
                        ""
                    }
                );
                let text = if matches!(column, Column::Name | Column::Provider) {
                    cell(&label, width as usize)
                } else {
                    format!("{label:>width$}", width = width as usize)
                };
                let rect = Rect::new(x, hdr.y, width, 1);
                Paragraph::new(text)
                    .style(if active {
                        th.accented().bold()
                    } else {
                        th.dimmed().bold()
                    })
                    .wrap(WrapMode::None)
                    .render(rect, frame);
                self.rects.borrow_mut().headers.push((column, rect));
                x = x.saturating_add(width + 1);
            }
            let elapsed = self.marquee_elapsed();
            self.view_rows(
                MouseList::Results,
                rows[1],
                self.search_results.len(),
                |i| {
                    layout::fingerprint(&(
                        &self.search_results[i],
                        self.rating_for(&self.search_results[i]),
                        self.wire_result(&self.search_results[i]),
                        self.search_mark_state(&self.search_results[i]),
                        extended,
                    ))
                },
                |i| {
                    let mut result = self.search_results[i].clone();
                    let (marked, adding, added) = self.search_mark_state(&result);
                    let state = if added { "+ In list · " } else if adding { "Adding… · " } else { "" };
                    result.file_name = format!("{}{state}{}", if marked { "✓ " } else { "" }, result.file_name);
                    search_line(
                        &result,
                        self.rating_for(&self.search_results[i]),
                        inner.width.saturating_sub(2),
                        extended,
                        th,
                        hovered_for(&self.search_list, i, elapsed),
                    )
                },
                frame,
            );
        }

        self.view_search_card(cols[1], frame);
    }

    fn view_search_card(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let th = &self.theme;
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(th.border())
            .title(" Details ");
        let inner = block.inner(area);
        block.render(area, frame);
        if let Some(r) = self.selected_result() {
            let kv = |k: &str, v: String| {
                Line::from_spans([
                    Span::styled(format!("{k:<10}"), th.dimmed()),
                    Span::styled(v, th.text()),
                ])
            };
            let mut lines = vec![
                Line::styled(r.file_name.clone(), th.title()),
                Line::raw(""),
                kv(
                    "size",
                    if r.file_size >= 0 {
                        human_bytes(r.file_size)
                    } else {
                        "unknown".into()
                    },
                ),
            ];
            let peers = [(r.seeders, "seeds"), (r.leechers, "leechers")]
                .into_iter()
                .filter(|(count, _)| *count >= 0)
                .map(|(count, label)| format!("{count} {label}"))
                .collect::<Vec<_>>()
                .join(", ");
            if !peers.is_empty() {
                lines.push(kv("peers", peers));
            }
            match self.search_mark_state(r) {
                (_, _, true) => lines.push(Line::styled("+ In your list", th.ok())),
                (_, true, _) => lines.push(Line::styled("Adding…", th.dimmed())),
                _ => {}
            }
            if !self.search_marked.is_empty() {
                lines.push(Line::styled(format!("{} selected · Enter downloads · Esc clears", self.search_marked.len()), th.accented()));
            }
            lines.extend([
                kv(
                    "uploaded",
                    if r.pub_date > 0 {
                        ymd(r.pub_date)
                    } else {
                        "unknown".into()
                    },
                ),
                kv("source", format!("{}  {}", r.engine, r.site_url)),
                Line::raw(""),
            ]);
            let inner = pad(inner);
            let rows = Flex::vertical()
                .constraints([Constraint::Fixed(7), Constraint::Fill])
                .split(inner);
            Paragraph::new(Text::from_lines(lines))
                .wrap(WrapMode::Word)
                .render(rows[0], frame);
            self.view_imdb_card(&r.file_name, rows[1], frame);
        }
    }

    pub(super) fn view_browse(&self, left: Rect, right: Rect, frame: &mut Frame) {
        let th = &self.theme;
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(th.border_focused())
            .padding(Sides::all(0))
            .title(" Browse providers ");
        let inner = block.inner(left);
        block.render(left, frame);
        let w = inner.width.saturating_sub(2) as usize;
        let elapsed = self.marquee_elapsed();
        self.view_rows(
            MouseList::Browse,
            inner,
            self.browse_entries.len(),
            |i| {
                layout::fingerprint(&(
                    self.browse_entries[i].provider,
                    self.browse_entries[i].label,
                ))
            },
            |i| {
                let entry = &self.browse_entries[i];
                let provider = if i > 0 && self.browse_entries[i - 1].provider == entry.provider {
                    ""
                } else {
                    entry.provider
                };
                let is_site = matches!(entry.source, browse::Source::Site(_));
                let label = format!("{} {}", if is_site { "↗" } else { "≡" }, entry.label);
                Line::from_spans([
                    Span::styled(cell(provider, 16), th.accented()),
                    Span::styled(
                        readable_cell(
                            &label,
                            w.saturating_sub(16),
                            hovered_for(&self.browse_list, i, elapsed),
                        ),
                        if is_site { th.dimmed() } else { th.text() },
                    ),
                ])
            },
            frame,
        );

        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(th.border())
            .title(" About ");
        let inner = block.inner(right);
        block.render(right, frame);
        let sel = self
            .browse_list
            .borrow()
            .selected()
            .and_then(|i| self.browse_entries.get(i));
        let text = match sel.map(|e| e.provider) {
            Some("1337x") => "1337x sits behind a browser challenge, so its lists cannot be loaded here.\nEach entry opens the page in your browser.",
            Some("YTS") => "Latest YTS releases (1080p picked per film) via the YTS JSON API.\nThe main domain does not resolve from this network; mirrors are tried, otherwise use Open site.",
            Some("EZTV") => "The 100 most recent TV episodes on EZTV, sorted by seeders.\nEntries carry IMDb ids, so the poster card appears as you move over them.",
            Some("The Pirate Bay") => "Top-100 lists from The Pirate Bay's public JSON feed.\nEntries carry IMDb ids, so the poster card appears as you move over them.",
            _ => "",
        };
        let mut lines = vec![Line::styled(text.to_string(), th.text()), Line::raw("")];
        if let Some(l) = &self.browse_loading {
            lines.push(Line::styled(
                format!("{} loading {l}…", SPINNER[self.tick % 4]),
                th.warn(),
            ));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from_spans([
            Span::styled("Enter", th.key()),
            Span::styled(" load list or open site   ", th.dimmed()),
            Span::styled("Esc", th.key()),
            Span::styled(" back to results", th.dimmed()),
        ]));
        Paragraph::new(Text::from_lines(lines))
            .wrap(WrapMode::Word)
            .render(pad(inner), frame);
    }
}

fn search_tail_width(extended: bool) -> usize {
    1 + 9 + 1 + 6 + 1 + 5 + if extended { 1 + 10 + 1 + 14 + 1 + 6 } else { 0 }
}
#[cfg(test)]
pub(super) fn result_line(
    r: &SearchResult,
    rating: Option<f32>,
    width: u16,
    th: &Theme,
    hover: Option<Duration>,
) -> Line<'static> {
    search_line(
        r,
        rating,
        width,
        layout::Tier::at(width).extended(),
        th,
        hover,
    )
}
fn search_line(
    r: &SearchResult,
    rating: Option<f32>,
    width: u16,
    extended: bool,
    th: &Theme,
    hover: Option<Duration>,
) -> Line<'static> {
    let name = usize::from(width).saturating_sub(search_tail_width(extended));
    let size = if r.file_size >= 0 {
        human_bytes(r.file_size)
    } else {
        "—".into()
    };
    let seeds = if r.seeders < 0 {
        String::new()
    } else {
        r.seeders.to_string()
    };
    let stars = rating
        .map(|v| format!("{v:.1}"))
        .unwrap_or_else(|| "—".into());
    let mut spans = vec![
        Span::styled(readable_cell(&r.file_name, name, hover), th.text()),
        Span::raw(" "),
        Span::styled(format!("{size:>9}"), th.text()),
        Span::raw(" "),
        Span::styled(format!("{seeds:>6}"), th.text()),
        Span::raw(" "),
        Span::styled(format!("{stars:>5}"), th.dimmed()),
    ];
    if extended {
        let peers = if r.leechers < 0 {
            String::new()
        } else {
            r.leechers.to_string()
        };
        spans.push(Span::styled(
            format!(
                " {:>10} {} {:>6}",
                ymd(r.pub_date),
                cell(&r.engine, 14),
                peers
            ),
            th.dimmed(),
        ));
    }
    Line::from_spans(spans)
}
