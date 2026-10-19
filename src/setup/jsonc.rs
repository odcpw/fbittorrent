//! Small JSONC editor: retain unrelated bytes/comments, replace owned root keys.
use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

fn string_end(bytes: &[u8], start: usize) -> Result<usize> {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Ok(i + 1),
            _ => i += 1,
        }
    }
    bail!("unterminated JSONC string")
}

fn clean(text: &str) -> Result<Vec<u8>> {
    if text.len() > 1_048_576 {
        bail!("JSONC settings exceed 1 MiB");
    }
    let mut bytes = text.as_bytes().to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i = string_end(&bytes, i)?;
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                bytes[i] = b' ';
                i += 1;
            }
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
            while i + 1 < bytes.len() && &bytes[i..i + 2] != b"*/" {
                bytes[i] = b' ';
                i += 1;
            }
            if i + 1 >= bytes.len() {
                bail!("unterminated JSONC comment");
            }
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
        } else {
            i += 1;
        }
    }
    Ok(bytes)
}

pub(super) fn parse(text: &str) -> Result<Value> {
    let mut bytes = clean(text)?;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            i = string_end(&bytes, i)?;
            continue;
        }
        if bytes[i] == b',' {
            let next = bytes[i + 1..].iter().find(|v| !v.is_ascii_whitespace());
            if matches!(next, Some(b'}' | b']')) {
                bytes[i] = b' ';
            }
        }
        i += 1;
    }
    serde_json::from_slice(&bytes).context("invalid JSONC settings")
}

/// Byte spans in an already validated JSONC document, including separators.
/// Keep comments/whitespace out of replacement ranges when removing an item.
pub(super) struct Entry {
    pub key: Option<String>,
    pub value: std::ops::Range<usize>,
    pub comma: Option<usize>,
}
pub(super) struct Syntax {
    bytes: Vec<u8>,
    root: std::ops::Range<usize>,
}
impl Syntax {
    pub fn new(text: &str) -> Result<Self> {
        parse(text)?;
        let bytes = clean(text)?;
        let start = bytes
            .iter()
            .position(|b| !b.is_ascii_whitespace())
            .context("empty JSONC")?;
        let end = value_end(&bytes, start)?;
        Ok(Self {
            bytes,
            root: start..end,
        })
    }
    pub fn is_array(&self, span: &std::ops::Range<usize>) -> bool {
        self.bytes[span.start] == b'['
    }
    pub fn at(&self, path: &[&str]) -> Result<Option<std::ops::Range<usize>>> {
        let mut span = self.root.clone();
        for key in path {
            if self.bytes[span.start] != b'{' {
                bail!("JSONC path must be an object");
            }
            let mut matching = self
                .members(span)?
                .into_iter()
                .filter(|entry| entry.key.as_deref() == Some(*key));
            let Some(entry) = matching.next() else {
                return Ok(None);
            };
            if matching.next().is_some() {
                bail!("ambiguous duplicate JSONC key {key}");
            }
            span = entry.value;
        }
        Ok(Some(span))
    }
    pub fn members(&self, span: std::ops::Range<usize>) -> Result<Vec<Entry>> {
        let bytes = &self.bytes;
        let object = bytes[span.start] == b'{';
        anyhow::ensure!(
            object || bytes[span.start] == b'[',
            "JSONC collection required"
        );
        let mut entries = Vec::new();
        let mut i = span.start + 1;
        loop {
            while bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i == span.end - 1 {
                break;
            }
            let key = if object {
                let end = string_end(bytes, i)?;
                let key = serde_json::from_slice(&bytes[i..end])?;
                i = end;
                while bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                anyhow::ensure!(bytes[i] == b':', "missing JSONC colon");
                i += 1;
                while bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                Some(key)
            } else {
                None
            };
            let start = i;
            i = value_end(bytes, i)?;
            let value = start..i;
            while bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let comma = if bytes[i] == b',' {
                let comma = i;
                i += 1;
                Some(comma)
            } else {
                None
            };
            entries.push(Entry { key, value, comma });
        }
        Ok(entries)
    }
}
fn value_end(bytes: &[u8], start: usize) -> Result<usize> {
    if bytes[start] == b'"' {
        return string_end(bytes, start);
    }
    if matches!(bytes[start], b'{' | b'[') {
        let mut depth = 0;
        let mut i = start;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => {
                    i = string_end(bytes, i)?;
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(i + 1);
                    }
                }
                _ => {}
            }
            i += 1;
        }
        bail!("unterminated JSONC value");
    }
    Ok(start
        + bytes[start..]
            .iter()
            .take_while(|b| !b.is_ascii_whitespace() && !matches!(b, b',' | b']' | b'}'))
            .count())
}

pub(super) fn edit(
    text: &str,
    owned: impl Fn(&str) -> bool,
    values: &Map<String, Value>,
) -> Result<String> {
    if !parse(text)?.is_object() {
        bail!("JSONC root must be an object");
    }
    let bytes = clean(text)?;
    let start = bytes.iter().position(|v| !v.is_ascii_whitespace()).unwrap();
    let mut i = start + 1;
    let mut ranges = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        if bytes[i] == b'}' {
            break;
        }
        let key_start = i;
        let key_end = string_end(&bytes, i)?;
        let key: String = serde_json::from_slice(&bytes[i..key_end])?;
        i = key_end;
        while bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes[i] != b':' {
            bail!("missing JSONC colon");
        }
        i += 1;
        let mut depth = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'"' => {
                    i = string_end(&bytes, i)?;
                    continue;
                }
                b'{' | b'[' => depth += 1,
                b'}' if depth == 0 => break,
                b'}' | b']' => depth -= 1,
                b',' if depth == 0 => {
                    i += 1;
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        if owned(&key) {
            ranges.push(key_start..i);
        }
    }
    let mut output = text.to_owned();
    for range in ranges.into_iter().rev() {
        output.replace_range(range, "");
    }
    let clean = clean(&output)?;
    let end = clean
        .iter()
        .rposition(|v| *v == b'}')
        .context("missing root close")?;
    if !values.is_empty() {
        let last = clean[..end]
            .iter()
            .rposition(|v| !v.is_ascii_whitespace())
            .unwrap();
        let comma = if matches!(clean[last], b'{' | b',') {
            ""
        } else {
            ","
        };
        let entries = values
            .iter()
            .map(|(key, value)| format!("  {}: {}", serde_json::to_string(key).unwrap(), value))
            .collect::<Vec<_>>()
            .join(",\n");
        output.insert_str(end, &format!("{comma}\n{entries}\n"));
    }
    parse(&output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    #[test]
    fn comments_urls_multiline_and_trailing_commas_survive() {
        let input = "{\n// keep\n\"other\": {\"url\":\"https://x//y\",\"list\":[1,2,],},\n\"torrents.old\": {\n\"action\":\"old\"\n},\n}";
        let values = serde_json::from_value(
            serde_json::json!({"torrents.open":{"action":"fbittorrent dashboard"}}),
        )
        .unwrap();
        let edited = super::edit(input, |key| key.starts_with("torrents"), &values).unwrap();
        assert!(edited.contains("// keep\n\"other\": {\"url\":\"https://x//y\",\"list\":[1,2,],}"));
        assert_eq!(
            super::parse(&edited).unwrap()["torrents.open"]["action"],
            "fbittorrent dashboard"
        );
        assert!(super::parse("{/* unterminated").is_err());
    }
}
