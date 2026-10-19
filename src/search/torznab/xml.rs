//! Torznab RSS -> the common search model. Unknown measurements stay unknown.
use super::*;
use crate::api::validation::Validate;
use quick_xml::{events::Event, Reader};
#[derive(Default)]
struct Item {
    title: String,
    link: String,
    enclosure: String,
    magnet: String,
    hash: String,
    comments: String,
    size: Option<i64>,
    seeds: Option<i64>,
    date: String,
}
pub(super) fn parse(bytes: &[u8], indexer: &Indexer) -> Result<Vec<SearchResult>> {
    anyhow::ensure!(
        bytes.len() <= RESPONSE_CAP as usize,
        "Torznab XML exceeds 4 MiB"
    );
    let mut reader = Reader::from_reader(bytes);
    let mut item = None::<Item>;
    let mut tag = Vec::new();
    let mut rows = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|_| anyhow!("Torznab returned invalid XML"))?
        {
            Event::DocType(_) => bail!("Torznab XML document types are not supported"),
            Event::Empty(e) => attributes(item.as_mut(), &e, reader.decoder())?,
            Event::Start(e) => {
                anyhow::ensure!(tag.len() < 64, "Torznab XML exceeds nesting limit");
                let name = e.local_name().as_ref().to_ascii_lowercase();
                if name == b"item" {
                    item = Some(Item::default());
                }
                attributes(item.as_mut(), &e, reader.decoder())?;
                tag.push(name);
            }
            Event::Text(e) => {
                let decoded = e
                    .decode()
                    .map_err(|_| anyhow!("Torznab XML text encoding failed"))?;
                let value = quick_xml::escape::unescape(&decoded)
                    .map_err(|_| anyhow!("Torznab XML text escape failed"))?;
                text(item.as_mut(), tag.last().map(Vec::as_slice), &value);
            }
            Event::CData(e) => {
                let value = e
                    .decode()
                    .map_err(|_| anyhow!("Torznab XML CDATA encoding failed"))?;
                text(item.as_mut(), tag.last().map(Vec::as_slice), &value);
            }
            Event::GeneralRef(e) => {
                let name = e
                    .decode()
                    .map_err(|_| anyhow!("Torznab XML reference failed"))?;
                let value = quick_xml::escape::unescape(&format!("&{name};"))
                    .map_err(|_| anyhow!("Torznab XML reference failed"))?
                    .into_owned();
                text(item.as_mut(), tag.last().map(Vec::as_slice), &value);
            }
            Event::End(e) => {
                if e.local_name().as_ref() == b"item" {
                    if let Some(value) = item.take().and_then(|i| finish(i, indexer)) {
                        if rows.len() < 5000 {
                            rows.push(value);
                        }
                    }
                }
                tag.pop();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    anyhow::ensure!(tag.is_empty(), "Torznab XML ended inside an element");
    Ok(rows)
}
fn attributes(
    item: Option<&mut Item>,
    element: &quick_xml::events::BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
) -> Result<()> {
    if !matches!(element.local_name().as_ref(), b"attr" | b"enclosure") {
        return Ok(());
    }
    let Some(row) = item else {
        return Ok(());
    };
    let mut name = String::new();
    let mut value = String::new();
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|_| anyhow!("Torznab returned invalid XML attributes"))?;
        let text = attribute
            .decode_and_unescape_value(decoder)
            .map_err(|_| anyhow!("Torznab returned invalid XML attribute"))?
            .into_owned();
        match attribute.key.as_ref() {
            b"name" => name = text,
            b"value" => value = text,
            b"url" => row.enclosure = text,
            b"length" => row.size = text.parse::<i64>().ok().filter(|v| *v >= 0),
            _ => {}
        }
    }
    match name.to_ascii_lowercase().as_str() {
        "seeders" | "seeds" => row.seeds = value.parse::<i64>().ok().filter(|v| *v >= 0),
        "size" => row.size = value.parse::<i64>().ok().filter(|v| *v >= 0),
        "magneturl" => row.magnet = value,
        "infohash" => row.hash = value,
        _ => {}
    }
    Ok(())
}
fn text(item: Option<&mut Item>, tag: Option<&[u8]>, text: &str) {
    if let Some(item) = item {
        match tag {
            Some(b"title") => item.title.push_str(text),
            Some(b"link") => item.link.push_str(text),
            Some(b"comments") => item.comments.push_str(text),
            Some(b"pubdate") => item.date.push_str(text),
            Some(b"size") => item.size = text.parse::<i64>().ok().filter(|v| *v >= 0),
            _ => {}
        }
    }
}
fn finish(i: Item, indexer: &Indexer) -> Option<SearchResult> {
    let link = if !i.magnet.is_empty() {
        i.magnet
    } else if i.hash.len() == 40 && i.hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        format!("magnet:?xt=urn:btih:{}", i.hash.to_ascii_lowercase())
    } else if !i.enclosure.is_empty() {
        i.enclosure
    } else {
        i.link
    };
    let parsed = url::Url::parse(&link)
        .or_else(|_| url::Url::parse(&indexer.url)?.join(&link))
        .ok()?;
    if !matches!(parsed.scheme(), "magnet" | "http" | "https") || i.title.trim().is_empty() {
        return None;
    }
    let row = SearchResult {
        file_name: i.title.trim().chars().take(1024).collect(),
        file_url: parsed.into(),
        file_size: i.size.unwrap_or(-1),
        seeders: i.seeds.unwrap_or(-1),
        leechers: -1,
        site_url: indexer.url.clone(),
        descr_link: i.comments,
        engine: indexer.name.clone(),
        imdb: String::new(),
        pub_date: crate::feed_date::parse_rfc2822(&i.date).max(0),
    };
    super::super::normalize::item(&row).validate().ok()?;
    Some(row)
}
