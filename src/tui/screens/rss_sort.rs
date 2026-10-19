//! RSS sort preferences survive item pushes and preserve the selected article.
use super::*;
use std::cmp::Ordering;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RssSort {
    Date,
    Name,
    Size,
}
impl RssSort {
    pub(super) fn compare(
        self,
        a: &crate::qbt::RssArticle,
        b: &crate::qbt::RssArticle,
        descending: bool,
    ) -> Ordering {
        let optional = |a: Option<i64>, b: Option<i64>| match (a, b) {
            (Some(a), Some(b)) => {
                if descending {
                    b.cmp(&a)
                } else {
                    a.cmp(&b)
                }
            }
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            _ => Ordering::Equal,
        };
        match self {
            Self::Date => optional((a.ts > 0).then_some(a.ts), (b.ts > 0).then_some(b.ts)),
            Self::Size => optional(a.size, b.size),
            Self::Name => {
                if descending {
                    b.title.cmp(&a.title)
                } else {
                    a.title.cmp(&b.title)
                }
            }
        }
        .then(a.id.cmp(&b.id))
    }
}
impl TuiApp {
    pub(super) fn sort_rss(&mut self, reverse: bool) -> Cmd<Msg> {
        let selected = self
            .selected_feed()
            .and_then(|f| {
                self.item_list
                    .borrow()
                    .selected()
                    .and_then(|i| f.articles.get(i))
            })
            .map(|a| a.id.clone());
        if reverse {
            self.rss_descending = !self.rss_descending;
        } else {
            self.rss_sort = match self.rss_sort {
                RssSort::Date => RssSort::Name,
                RssSort::Name => RssSort::Size,
                RssSort::Size => RssSort::Date,
            };
            self.rss_descending = self.rss_sort != RssSort::Name;
        }
        for feed in &mut self.feeds {
            feed.articles
                .sort_by(|a, b| self.rss_sort.compare(a, b, self.rss_descending));
        }
        let index = selected.and_then(|id| {
            self.selected_feed()
                .and_then(|f| f.articles.iter().position(|a| a.id == id))
        });
        self.item_list.borrow_mut().select(index);
        self.set_status(
            format!(
                "Sorted items by {} {}",
                match self.rss_sort {
                    RssSort::Date => "date",
                    RssSort::Name => "name",
                    RssSort::Size => "size",
                },
                if self.rss_descending { "↓" } else { "↑" }
            ),
            false,
        );
        Cmd::none()
    }
}
