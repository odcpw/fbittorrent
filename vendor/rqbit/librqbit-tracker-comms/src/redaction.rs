//! Credential-safe tracker URLs and diagnostics, before tracing or API projection.
use url::Url;

const REDACTED:&str="REDACTED";
fn parameter(value:&str)->bool {
    matches!(value.to_ascii_lowercase().as_str(),"passkey"|"key"|"token"|"api_key"|"apikey"|"authkey"|"password"|"secret")
}
fn decoded(value:&str)->String {
    let mut result=value.to_owned();
    for _ in 0..4 {
        let next=urlencoding::decode(&result).map(|s|s.into_owned()).unwrap_or_else(|_|result.clone());
        if next==result {break;}
        result=next;
    }
    result
}
fn credential(value:&str)->bool {
    let value=value.trim_matches(['[',']','(',')',',','.']);
    (value.to_ascii_lowercase().contains("passkey")&&!parameter(value))
        || (value.len()>=16 && value.bytes().all(|b|b.is_ascii_alphanumeric()||b"+-_/=".contains(&b)))
        // Deeply escaped input stays private after the bounded decode pass.
        ||value.as_bytes().windows(3).any(|w|w[0]==b'%'&&w[1].is_ascii_hexdigit()&&w[2].is_ascii_hexdigit())
}
fn path_parts(url:&Url)->Vec<(String,bool)> {
    let mut previous=false;
    url.path_segments().into_iter().flatten().map(|raw| {
        let value=decoded(raw);
        let named=value.split('/').any(|part|parameter(part))
            ||value.split([';','&','/']).any(|part|part.split_once('=').is_some_and(|(key,_)|parameter(key)));
        let private=previous||credential(&value)||named;
        previous=value.rsplit('/').next().is_some_and(parameter);
        (raw.to_owned(),private)
    }).collect()
}
fn remember(secrets:&mut Vec<String>,value:&str) {
    if value.is_empty(){return;}
    let mut value=value.to_owned();
    for _ in 0..5 {
        secrets.push(value.clone());
        secrets.push(value.chars().map(|ch|if ch.is_control(){' '}else{ch}).collect());
        if value.len()>=16&&value.bytes().all(|b|b.is_ascii_hexdigit()) {
            secrets.push(value.to_ascii_lowercase());secrets.push(value.to_ascii_uppercase());
        }
        let next=urlencoding::decode(&value).map(|s|s.into_owned()).unwrap_or_else(|_|value.clone());
        if next==value {break;}
        value=next;
    }
}
fn secrets(url:&Url)->Vec<String> {
    let mut values=Vec::new();
    for raw in [Some(url.username()),url.password(),url.fragment()].into_iter().flatten(){remember(&mut values,raw);}
    for pair in url.query().unwrap_or("").split('&') {
        if let Some((_,value))=pair.split_once('=') {remember(&mut values,value);remember(&mut values,&value.replace('+'," "));}
        else {remember(&mut values,pair);}
    }
    for (part,private) in path_parts(url) {
        if private {
            remember(&mut values,&part);remember(&mut values,decoded(&part).trim_matches(['[',']','(',')',',','.']));
            // Parse each escaping layer before decoding its values: %26 and
            // %3B can belong to a credential, rather than delimit parameters.
            let mut layer=part;
            for _ in 0..5 {
                let mut previous=false;
                for segment in layer.split('/') {
                    if previous {remember(&mut values,segment);}
                    previous=parameter(segment);
                    for item in segment.split([';','&']) {
                        if let Some((key,value))=item.split_once('=') {
                            if parameter(key) {remember(&mut values,value);}
                        }
                    }
                    // An encoded '=' may only become structural at this layer.
                    // Retain the entire suffix too, including encoded delimiters.
                    if let Some((key,value))=segment.split_once('=') {
                        if key.rsplit([';','&']).next().is_some_and(parameter) {remember(&mut values,value);}
                    }
                }
                let next=decoded(&layer);
                if next==layer {break;}layer=next;
            }
        }
    }
    values
}
pub fn redact_url(url:&Url)->String {
    let mut public=url.clone();
    let _=public.set_username("");let _=public.set_password(None);
    let path=path_parts(url).into_iter().map(|(part,private)|if private {REDACTED.into()}else{part}).collect::<Vec<String>>().join("/");
    public.set_path(&path);public.set_query(None);public.set_fragment(None);
    if url.query().is_some(){format!("{public}?…")}else{public.to_string()}
}

fn sanitize_urls(text:&str,private:&mut Vec<String>)->String {
    let mut output=String::with_capacity(text.len());
    let mut cursor=0;
    while cursor<text.len() {
        let Some(offset)=text[cursor..].find("://") else {output.push_str(&text[cursor..]);break;};
        let separator=cursor+offset;
        let start=text[cursor..separator].char_indices().rev()
            .take_while(|(_,ch)|ch.is_ascii_alphanumeric()||matches!(ch,'+'|'-'|'.'))
            .last().map_or(separator,|(i,_)|cursor+i);
        let end=text[separator+3..].char_indices().find(|(_,ch)|ch.is_whitespace()||matches!(ch,'"'|'\''|'<'|'>'|'('|')'|','))
            .map_or(text.len(),|(i,_)|separator+3+i);
        if start<separator {
            if let Ok(parsed)=Url::parse(&text[start..end]) {
                private.extend(secrets(&parsed));
                output.push_str(&text[cursor..start]);output.push_str(&redact_url(&parsed));cursor=end;continue;
            }
        }
        let next=separator+3;output.push_str(&text[cursor..next]);cursor=next;
    }
    output
}

/// Sanitize every URL token (including adjacent quoted URLs) and known values
/// echoed outside a URL. Never log a remote response before calling this helper.
pub fn sanitize(url:&Url,text:&str)->String {
    if text.len()>32768 {return "tracker diagnostic omitted (too long)".into();}
    let text=text.chars().map(|ch|if ch.is_control(){' '}else{ch}).collect::<String>();
    let mut private=secrets(url);
    let output=sanitize_urls(&text,&mut private);
    let output=sanitize_urls(&decoded(&output),&mut private);
    // Prose parameters also carry credentials even without a URL or known base.
    let mut cursor=0;
    while let Some(offset)=output[cursor..].find('=') {
        let equals=cursor+offset;
        let start=output[cursor..equals].char_indices().rev().take_while(|(_,ch)|ch.is_ascii_alphanumeric()||*ch=='_').last().map_or(equals,|(i,_)|cursor+i);
        cursor=equals+1;
        if parameter(&output[start..equals]) {
            let quote=output[cursor..].chars().next().filter(|ch|matches!(ch,'\"'|'\''));
            let begin=cursor+usize::from(quote.is_some());
            let end=output[begin..].char_indices().find(|(_,ch)|quote.map_or_else(||ch.is_whitespace()||matches!(ch,'&'|';'|','|'\"'|'\''|')'|']'|'}'),|q|*ch==q)).map_or(output.len(),|(i,_)|begin+i);
            remember(&mut private,&output[begin..end]);cursor=end;
        }
    }
    let mut safe=output;
    private.sort_by_key(|s|std::cmp::Reverse(s.len()));private.dedup();
    for value in private.into_iter().filter(|s|!s.is_empty()){safe=safe.replace(&value,"…");}
    safe.chars().map(|ch|if ch.is_control(){' '}else{ch}).take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_credentials_and_encoded_echoes_never_leave_diagnostics() {
        for token in ["0123456789abcdef0123456789abcdef","AbCdEf0123456789+/==","SYNTHETIC_PATH_PASSKEY","abc123passkey"] {
            let encoded=urlencoding::encode(token).to_string();
            let url=Url::parse(&format!("https://tracker.invalid/{encoded}/announce?passkey=%73hort")).unwrap();
            let public=redact_url(&url);
            assert!(public.contains("REDACTED/announce"));
            let message=sanitize(&url,&format!("{token} {encoded} %73hort short \"{url}\",https://other.invalid/{encoded}/announce"));
            for secret in [token,encoded.as_str(),"%73hort","short"] {assert!(!message.contains(secret),"{message}");}
        }
    }
    #[test]
    fn named_short_path_values_and_prose_parameters_are_private() {
        for path in ["passkey/abc/announce","key=abc/announce","announce;token=abc","%70asskey/%61bc/announce"] {
            let url=Url::parse(&format!("https://tracker.invalid/{path}")).unwrap();
            assert!(!redact_url(&url).contains("abc"));
            let text=sanitize(&url,"abc token=xyz Key=123 PASSKEY=qrs");
            for value in ["abc","xyz","123","qrs"] {assert!(!text.contains(value),"{text}");}
        }
        for path in ["passkey%2Fabc/announce","passkey%252Fabc/announce","key=abc%26def/announce","announce;token=abc%3Bdef"] {
            let url=Url::parse(&format!("https://tracker.invalid/{path}")).unwrap();
            assert!(!redact_url(&url).contains("abc"));
            let echo=if path.contains("%26") {"abc&def abc%26def"}else if path.contains("%3B") {"abc;def abc%3Bdef"}else{"abc"};
            let text=sanitize(&url,echo);
            assert!(!text.contains("abc")&&!text.contains("def"),"{text}");
        }
        assert_eq!(redact_url(&Url::parse("udp://tracker.invalid:80/announce").unwrap()),"udp://tracker.invalid:80/announce");
    }
    #[test]
    fn punctuation_ipv6_and_alternate_encoded_echoes_stay_private() {
        let url=Url::parse("https://tracker.invalid/passkey/a%26bc/announce?token=quoted%26value").unwrap();
        let text=sanitize(&url,"a%26bc a&bc token=quoted&value token=\"other&value\" (http://[::1]/0123456789abcdef0123456789abcdef/announce)");
        for secret in ["a%26bc","a&bc","quoted","other&value","0123456789abcdef0123456789abcdef"] {assert!(!text.contains(secret),"{text}");}
        let text=sanitize(&url,"key=%71%75%6fted%26value");
        assert!(!text.contains("quoted")&&!text.contains("value"),"{text}");
    }
}
