use anyhow::Result;
use serde_json::{Map, Value, json};

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
    let mut found = false;
    for section in layout.values_mut() {
        if let Some(items) = section.as_array_mut() {
            found |= items.iter().any(|item| item == "oliver.torrents");
            if uninstall {
                items.retain(|item| item != "oliver.torrents");
            }
        }
    }
    if !uninstall && !found {
        let right = layout
            .entry("right")
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("bar.layout.right must be an array"))?;
        right.push(json!("oliver.torrents"));
    }
    if (uninstall && !found) || (!uninstall && found) {
        return Ok(text.to_owned());
    }
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
        assert!(
            super::super::jsonc::parse(&removed)
                .unwrap()
                .get("torrents")
                .is_none()
        );
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
}
