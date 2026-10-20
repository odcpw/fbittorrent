//! Minimal bencode: writers for the few messages the seed emits and a parser
//! that keeps raw spans (the info hash is the SHA-1 of the exact info bytes).

use std::ops::Range;

pub enum Value {
    Int(i64),
    Bytes(Range<usize>),
    List(Vec<(Value, Range<usize>)>),
    Dict(Vec<(Range<usize>, Value, Range<usize>)>),
}

pub struct Parsed<'a> {
    pub data: &'a [u8],
    pub root: Value,
}

fn parse_at(data: &[u8], pos: &mut usize, depth: usize) -> Result<Value, String> {
    if depth > 32 {
        return Err("bencode nesting too deep".into());
    }
    let byte = *data.get(*pos).ok_or("truncated bencode")?;
    match byte {
        b'i' => {
            let end = data[*pos..].iter().position(|b| *b == b'e').ok_or("unterminated int")? + *pos;
            let text = std::str::from_utf8(&data[*pos + 1..end]).map_err(|e| e.to_string())?;
            *pos = end + 1;
            Ok(Value::Int(text.parse().map_err(|e: std::num::ParseIntError| e.to_string())?))
        }
        b'l' => {
            *pos += 1;
            let mut items = Vec::new();
            while *data.get(*pos).ok_or("unterminated list")? != b'e' {
                let start = *pos;
                let value = parse_at(data, pos, depth + 1)?;
                items.push((value, start..*pos));
            }
            *pos += 1;
            Ok(Value::List(items))
        }
        b'd' => {
            *pos += 1;
            let mut items = Vec::new();
            while *data.get(*pos).ok_or("unterminated dict")? != b'e' {
                let Value::Bytes(key) = parse_at(data, pos, depth + 1)? else {
                    return Err("dict key is not a string".into());
                };
                let start = *pos;
                let value = parse_at(data, pos, depth + 1)?;
                items.push((key, value, start..*pos));
            }
            *pos += 1;
            Ok(Value::Dict(items))
        }
        b'0'..=b'9' => {
            let colon = data[*pos..].iter().position(|b| *b == b':').ok_or("bad string")? + *pos;
            let len: usize = std::str::from_utf8(&data[*pos..colon])
                .map_err(|e| e.to_string())?
                .parse()
                .map_err(|e: std::num::ParseIntError| e.to_string())?;
            let start = colon + 1;
            let end = start.checked_add(len).filter(|end| *end <= data.len()).ok_or("string overruns input")?;
            *pos = end;
            Ok(Value::Bytes(start..end))
        }
        other => Err(format!("unexpected bencode byte {other:#x}")),
    }
}

pub fn parse(data: &[u8]) -> Result<Parsed<'_>, String> {
    let mut pos = 0;
    let root = parse_at(data, &mut pos, 0)?;
    Ok(Parsed { data, root })
}

impl Value {
    /// Value and raw span of a dict member.
    pub fn get<'v>(&'v self, data: &[u8], key: &[u8]) -> Option<(&'v Value, Range<usize>)> {
        let Value::Dict(items) = self else { return None };
        items
            .iter()
            .find(|(k, _, _)| &data[k.clone()] == key)
            .map(|(_, value, span)| (value, span.clone()))
    }

    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub fn bytes<'d>(&self, data: &'d [u8]) -> Option<&'d [u8]> {
        match self {
            Value::Bytes(range) => Some(&data[range.clone()]),
            _ => None,
        }
    }
}

pub fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(bytes.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(bytes);
}

pub fn put_int(out: &mut Vec<u8>, value: i64) {
    out.push(b'i');
    out.extend_from_slice(value.to_string().as_bytes());
    out.push(b'e');
}

#[cfg(test)]
mod tests {
    #[test]
    fn spans_and_lookup() {
        let data = b"d4:infod6:lengthi5e4:name1:xe1:zli1ei2eee";
        let parsed = super::parse(data).unwrap();
        let (info, span) = parsed.root.get(data, b"info").unwrap();
        assert_eq!(&data[span], b"d6:lengthi5e4:name1:xe");
        assert_eq!(info.get(data, b"length").unwrap().0.int(), Some(5));
        assert_eq!(info.get(data, b"name").unwrap().0.bytes(data), Some(&b"x"[..]));
        assert!(super::parse(b"d3:abc").is_err());
    }
}
