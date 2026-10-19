//! Small JSONC editor: retain unrelated bytes/comments, replace owned root keys.
use anyhow::{Context, Result, bail};
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
