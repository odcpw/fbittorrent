use anyhow::Result;
use serde_json::{json, Map, Value};

/// Older installs added a Torrents submenu alongside the desktop application.
pub(super) fn remove_legacy_launchers(text: &str) -> Result<String> {
    let current = super::jsonc::parse(text)?;
    // These generic IDs can also be used for personal shortcuts.
    super::jsonc::edit(
        text,
        |key| {
            let entry = &current[key];
            let action = entry["action"].as_str().unwrap_or_default();
            match key {
                "torrent" => action == "fbittorrent dashboard",
                "torrents" => {
                    action.is_empty()
                        && entry["aliases"].as_array().is_some_and(|aliases| {
                            aliases.iter().any(|alias| alias == "fbittorrent")
                        })
                }
                "torrents.open" | "torrents.add" | "torrents.search" | "torrents.stop"
                | "torrents.tui" => action
                    .split_ascii_whitespace()
                    .any(|word| word == "fbittorrent"),
                _ => false,
            }
        },
        &Map::new(),
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
    fn removes_old_menu_and_dashboard_shortcut_without_claiming_other_shortcuts() {
        let old = r#"{
          // owner's shortcuts
          "other":{"label":"Keep"},
          "torrent":{"action":"fbittorrent dashboard"},
          "torrents":{"label":"Torrents","aliases":["torrent","fbittorrent"]},
          "torrents.add":{"action":"fbittorrent add-clipboard"},
          "torrents.stop":{"when":"systemctl qbittorrent","action":"fbittorrent down"},
        }"#;
        let cleaned = super::remove_legacy_launchers(old).unwrap();
        assert_eq!(
            super::super::jsonc::parse(&cleaned).unwrap(),
            serde_json::json!({"other":{"label":"Keep"}})
        );
        assert!(cleaned.contains("// owner's shortcuts"));
        assert_eq!(super::remove_legacy_launchers(&cleaned).unwrap(), cleaned);

        let custom = r#"{"torrent":{"action":"my-torrent-app"},"torrents":{"label":"My tools"},"torrents.open":{"action":"open-something-else"}}"#;
        assert_eq!(super::remove_legacy_launchers(custom).unwrap(), custom);
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
