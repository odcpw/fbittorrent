//! Typed search comparisons; absent measurements stay last in either direction.
use super::sort::{direction, optional};
use super::*;
use std::cmp::Ordering;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(in crate::tui) enum SearchSort {
    Name,
    Size,
    Seeds,
    Rating,
    Uploaded,
    Provider,
    Peers,
}
pub(in crate::tui) fn search_order(
    a: &api::SearchItem,
    b: &api::SearchItem,
    field: SearchSort,
    descending: bool,
) -> Ordering {
    let order = match field {
        SearchSort::Name => direction(a.name.cmp(&b.name), descending),
        SearchSort::Size => optional(a.size, b.size, descending, u64::cmp),
        SearchSort::Seeds => optional(a.seeds, b.seeds, descending, u64::cmp),
        SearchSort::Rating => optional(a.rating, b.rating, descending, f64::total_cmp),
        SearchSort::Uploaded => optional(a.uploaded_at, b.uploaded_at, descending, u64::cmp),
        SearchSort::Provider => direction(a.providers.cmp(&b.providers), descending),
        SearchSort::Peers => optional(a.leechers, b.leechers, descending, u64::cmp),
    };
    order.then(a.name.cmp(&b.name)).then(a.key.cmp(&b.key))
}
pub(in crate::tui) fn display_search_order(
    a: &crate::qbt::SearchResult,
    b: &crate::qbt::SearchResult,
    ratings: (Option<f32>, Option<f32>),
    field: SearchSort,
    descending: bool,
) -> Ordering {
    let order = match field {
        SearchSort::Name => direction(a.file_name.cmp(&b.file_name), descending),
        SearchSort::Size => optional(
            (a.file_size >= 0).then_some(a.file_size),
            (b.file_size >= 0).then_some(b.file_size),
            descending,
            i64::cmp,
        ),
        SearchSort::Seeds => optional(
            (a.seeders >= 0).then_some(a.seeders),
            (b.seeders >= 0).then_some(b.seeders),
            descending,
            i64::cmp,
        ),
        SearchSort::Rating => optional(ratings.0, ratings.1, descending, f32::total_cmp),
        SearchSort::Uploaded => optional(
            (a.pub_date > 0).then_some(a.pub_date),
            (b.pub_date > 0).then_some(b.pub_date),
            descending,
            i64::cmp,
        ),
        SearchSort::Provider => direction(a.engine.cmp(&b.engine), descending),
        SearchSort::Peers => optional(
            (a.leechers >= 0).then_some(a.leechers),
            (b.leechers >= 0).then_some(b.leechers),
            descending,
            i64::cmp,
        ),
    };
    order
        .then(a.file_name.cmp(&b.file_name))
        .then(a.file_url.cmp(&b.file_url))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_search_measurements_stay_last_and_zero_remains_known() {
        fn item(key: &str, size: Option<u64>) -> api::SearchItem {
            api::SearchItem {
                key: key.into(),
                name: key.into(),
                providers: vec!["p".into()],
                size,
                seeds: size,
                leechers: size,
                uploaded_at: size,
                rating: size.map(|s| s as f64),
                magnet: None,
                url: None,
                page_url: None,
                imdb_id: None,
                provider_url: None,
                added: None,
            }
        }
        for field in [
            SearchSort::Size,
            SearchSort::Seeds,
            SearchSort::Rating,
            SearchSort::Uploaded,
            SearchSort::Peers,
        ] {
            for descending in [false, true] {
                let mut rows = [
                    item("unknown", None),
                    item("zero", Some(0)),
                    item("large", Some(900)),
                ];
                rows.sort_by(|a, b| search_order(a, b, field, descending));
                assert_eq!(rows[2].key, "unknown");
                assert_eq!(rows[usize::from(descending)].key, "zero");
            }
        }
    }
}
