use anyhow::Result;
use serde_json::{json, Map, Value};

pub(super) fn edit(text: &str, uninstall: bool) -> Result<String> {
    let entries: Map<String, Value> = if uninstall {
        Map::new()
    } else {
        serde_json::from_value(json!({
            "torrents":{"icon":"󰇚","label":"Torrents","aliases":["torrent","fbittorrent"]},
            "torrents.open":{"icon":"▢","label":"Open","action":"fbittorrent dashboard"},
            "torrents.search":{"icon":"","label":"Search","action":"fbittorrent dashboard --view search"},
            "torrents.add":{"icon":"󰐕","label":"Add from clipboard","action":"fbittorrent add-clipboard"},
            "torrents.tui":{"icon":"󰆍","label":"Terminal","action":"omarchy-launch-or-focus-tui --app-id=org.omarchy.fbittorrent fbittorrent tui"},
            "torrents.stop":{"icon":"󰐥","label":"Stop fbittorrent","when":"fbittorrent status --quiet","action":"fbittorrent down"}
        }))?
    };
    let current = super::jsonc::parse(text)?;
    let mut desired = current.as_object().cloned().unwrap_or_default();
    desired.retain(|key, _| key != "torrents" && !key.starts_with("torrents."));
    desired.extend(entries.clone());
    if current == Value::Object(desired) {
        return Ok(text.to_owned());
    }
    super::jsonc::edit(
        text,
        |key| key == "torrents" || key.starts_with("torrents."),
        &entries,
    )
}

pub(super) fn bar(text: &str, uninstall: bool) -> Result<String> {
    let current = super::jsonc::parse(text)?;
    let mut bar = current.get("bar").cloned().unwrap_or(json!({}));
    if bar.get("layout").is_none() {
        bar["layout"] = json!({});
    }
    let layout = bar["layout"]
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("bar.layout must be an object"))?;
    let syntax = super::jsonc::Syntax::new(text)?;
    let mut found = false;
    let mut removals = Vec::new();
    if let Some(span) = syntax.at(&["bar", "layout"])? {
        // Walk source order so the owner's first placement and settings win.
        for section in syntax.members(span)? {
            if !syntax.is_array(&section.value) {
                continue;
            }
            let entries = syntax.members(section.value)?;
            let mut kept = Vec::new();
            for entry in &entries {
                let item = super::jsonc::parse(&text[entry.value.clone()])?;
                let own = item.as_str() == Some("oliver.torrents")
                    || item.get("id").and_then(Value::as_str) == Some("oliver.torrents");
                let keep = !own || (!uninstall && !found);
                found |= own;
                kept.push(keep);
                if !keep {
                    removals.push(entry.value.clone());
                    if let Some(comma) = entry.comma {
                        removals.push(comma..comma + 1);
                    }
                }
            }
            // When removing the tail of an ordinary JSON array, also remove
            // its preceding comma. Preserve an original JSONC trailing comma.
            if entries.last().is_some_and(|entry| entry.comma.is_none()) {
                if let Some(last) = kept.iter().rposition(|keep| *keep) {
                    if let Some(comma) = entries[last].comma {
                        removals.push(comma..comma + 1);
                    }
                }
            }
        }
    }
    if found || uninstall {
        let mut output = text.to_owned();
        removals.sort_unstable_by_key(|range| range.start);
        for range in removals.into_iter().rev() {
            output.replace_range(range, "");
        }
        super::jsonc::parse(&output)?;
        return Ok(output);
    }
    let right = layout
        .entry("right")
        .or_insert(json!([]))
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("bar.layout.right must be an array"))?;
    right.push(json!("oliver.torrents"));
    let values = Map::from_iter([("bar".to_owned(), bar)]);
    super::jsonc::edit(text, |key| key == "bar", &values)
}

#[cfg(test)]
mod tests {
    #[test]
    fn menu_jsonc_edit_idempotent() {
        let input = "{\n// kept comment\n\"other\":{\"label\":\"keep\"},\n\"torrents.stop\":{\"when\":\"systemctl qbittorrent\"},\n}";
        let once = super::edit(input, false).unwrap();
        assert_eq!(once, super::edit(&once, false).unwrap());
        assert_eq!(
            super::super::jsonc::parse(&once).unwrap()["torrents.stop"]["when"],
            "fbittorrent status --quiet"
        );
        assert!(once.contains("// kept comment"));
        let removed = super::edit(&once, true).unwrap();
        assert!(super::super::jsonc::parse(&removed)
            .unwrap()
            .get("torrents")
            .is_none());
        assert_eq!(
            super::super::jsonc::parse(&removed).unwrap()["other"]["label"],
            "keep"
        );
    }
    #[test]
    fn layout_keeps_existing_placement_and_others() {
        let initial = "{\"bar\":{\"layout\":{\"left\":[\"omarchy.clock\",\"oliver.torrents\"]}},\"other\":true}";
        assert_eq!(initial, super::bar(initial, false).unwrap());
        let removed = super::bar(initial, true).unwrap();
        assert_eq!(
            super::super::jsonc::parse(&removed).unwrap()["bar"]["layout"]["left"],
            serde_json::json!(["omarchy.clock"])
        );
        let installed = super::bar("{}", false).unwrap();
        assert_eq!(installed, super::bar(&installed, false).unwrap());
    }
    #[test]
    fn object_widget_is_present_without_changing_a_byte() {
        let text="{\r\n  \"bar\" : {\"layout\": {\"right\": [/*owner*/ {\"id\":\"oliver.torrents\",\"settings\":{\"keep\":true}},],}},\r\n  \"untouched\" : \"https://example.invalid//path\"\r\n}\r\n";
        assert_eq!(text, super::bar(text, false).unwrap());
    }
    #[test]
    fn duplicates_keep_the_first_source_placement_and_settings() {
        let text = r#"{
  "bar" : {"layout": {
    "right": [ {"id":"oliver.torrents", "settings":{"owner":true}}, "other", "other" ],
    "left": [/* keep this comment */ "oliver.torrents", {"id":"oliver.torrents"} ]
  }},
  "unrelated": {"note":"oliver.torrents"}
}"#;
        let edited = super::bar(text, false).unwrap();
        let expected = text.replace("\"oliver.torrents\", {\"id\":\"oliver.torrents\"}", " ");
        assert_eq!(edited, expected);
        let value = super::super::jsonc::parse(&edited).unwrap();
        assert_eq!(
            value["bar"]["layout"]["right"][0]["settings"]["owner"],
            true
        );
        assert_eq!(edited, super::bar(&edited, false).unwrap());
    }
    #[test]
    fn duplicate_removal_keeps_arrays_valid_at_every_position() {
        for input in [
            r#"["oliver.torrents","oliver.torrents","other"]"#,
            r#"["other","oliver.torrents","oliver.torrents"]"#,
            r#"["oliver.torrents","other","oliver.torrents",]"#,
            r#"["oliver.torrents",/* keep */{"id":"oliver.torrents"},"oliver.torrents"]"#,
        ] {
            let text = format!("{{\"bar\":{{\"layout\":{{\"right\":{input}}}}}}}");
            let edited = super::bar(&text, false).unwrap();
            let value = super::super::jsonc::parse(&edited).unwrap();
            let items = value["bar"]["layout"]["right"].as_array().unwrap();
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.as_str() == Some("oliver.torrents"))
                    .count(),
                1
            );
            assert_eq!(edited, super::bar(&edited, false).unwrap());
            if text.contains("/* keep */") {
                assert!(edited.contains("/* keep */"));
            }
        }
    }
    #[test]
    fn uninstall_removes_both_forms_but_not_nested_ids_or_other_duplicates() {
        let text = r#"{"bar":{"layout":{"right":["oliver.torrents",{"id":"other","settings":{"id":"oliver.torrents"}},"other","other",{"id":"oliver.torrents"}]}}}"#;
        let edited = super::bar(text, true).unwrap();
        let value = super::super::jsonc::parse(&edited).unwrap();
        assert_eq!(
            value["bar"]["layout"]["right"],
            serde_json::json!([{"id":"other","settings":{"id":"oliver.torrents"}},"other","other"])
        );
        assert_eq!(edited, super::bar(&edited, true).unwrap());
    }
}
