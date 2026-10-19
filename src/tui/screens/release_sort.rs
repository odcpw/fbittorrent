//! Release sorting runs on intent or pushed data, never during a frame.
use super::*;
use std::cmp::Ordering;
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReleaseSort {
    #[default]
    Date,
    Name,
    Size,
    Seeds,
    Quality,
}
impl ReleaseSort {
    fn next(self) -> Self {
        match self {
            Self::Date => Self::Name,
            Self::Name => Self::Size,
            Self::Size => Self::Seeds,
            Self::Seeds => Self::Quality,
            Self::Quality => Self::Date,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Name => "name",
            Self::Size => "size",
            Self::Seeds => "seeds",
            Self::Quality => "quality",
        }
    }
    pub(super) fn compare(self, a: &Episode, b: &Episode, descending: bool) -> Ordering {
        let optional = |a: i64, b: i64| match (a >= 0, b >= 0) {
            (true, true) => {
                if descending {
                    b.cmp(&a)
                } else {
                    a.cmp(&b)
                }
            }
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        };
        let order = match self {
            Self::Size => return optional(a.size, b.size).then(a.key.cmp(&b.key)),
            Self::Seeds => return optional(a.seeds, b.seeds).then(a.key.cmp(&b.key)),
            Self::Date => {
                return optional(
                    if a.released > 0 { a.released } else { -1 },
                    if b.released > 0 { b.released } else { -1 },
                )
                .then(a.key.cmp(&b.key))
            }
            Self::Name => a.title.cmp(&b.title),
            Self::Quality => {
                return optional(
                    a.quality.trim_end_matches('p').parse().unwrap_or(-1),
                    b.quality.trim_end_matches('p').parse().unwrap_or(-1),
                )
                .then(a.key.cmp(&b.key))
            }
        };
        (if descending { order.reverse() } else { order }).then(a.key.cmp(&b.key))
    }
}
impl TuiApp {
    pub(super) fn sort_releases(&mut self, reverse: bool) -> Cmd<Msg> {
        if reverse {
            self.release_descending = !self.release_descending;
        } else {
            self.release_sort = self.release_sort.next();
            self.release_descending =
                !matches!(self.release_sort, ReleaseSort::Name | ReleaseSort::Quality);
        }
        self.reorder_releases();
        self.set_status(
            format!(
                "Sorted releases by {} {}",
                self.release_sort.label(),
                if self.release_descending {
                    "↓"
                } else {
                    "↑"
                }
            ),
            false,
        );
        Cmd::none()
    }
    pub(super) fn reorder_releases(&mut self) {
        let selected = self.selected_release_target();
        let sort = self.release_sort;
        let descending = self.release_descending;
        for show in &mut self.shows.shows {
            show.episodes.sort_by(|a, b| sort.compare(a, b, descending));
        }
        self.rebuild_inbox();
        if let Some(target) = selected {
            let index = if self.shows_inbox() {
                self.inbox_rows.iter().position(|r| *r == target)
            } else {
                self.selected_show()
                    .filter(|s| s.imdb == target.imdb)
                    .and_then(|s| s.episodes.iter().position(|e| e.key == target.key))
            };
            self.ep_list.borrow_mut().select(index);
        }
    }
}
