//! Deterministic, backend-free exports of the same catalogue used by dispatch.
use super::*;
use serde_json::{json, Value};

fn surfaces(filter: &str) -> impl Iterator<Item = Surface> + '_ {
    [Surface::Tui, Surface::Dashboard, Surface::Popup]
        .into_iter()
        .filter(move |surface| filter == "all" || filter == surface.name())
}

pub(crate) fn keymap(filter: &str) -> Value {
    let actions: Vec<_> = all()
        .filter(|action| surfaces(filter).any(|s| action.keys_for(s).is_some()))
        .map(|action| {
            let keys: serde_json::Map<String, Value> = surfaces(filter)
                .map(|surface| {
                    (
                        surface.name().into(),
                        action.keys_for(surface).map_or(Value::Null, |keys| {
                            json!(keys.iter().map(|key| key.label()).collect::<Vec<_>>())
                        }),
                    )
                })
                .collect();
            let mut row = json!({"id":action.id,"label":action.doc_label,"scope":action.scope.name(),"keys":keys});
            if let Some(note) = action.surface_note {
                row["note"] = note.into();
            }
            row
        })
        .collect();
    json!({"version":1,"actions":actions})
}

pub(crate) fn markdown(filter: &str) -> String {
    let selected: Vec<_> = surfaces(filter).collect();
    let mut out = String::from("| Scope | Action |");
    for surface in &selected {
        out.push_str(&format!(" {} |", surface.name()));
    }
    out.push_str(" Note |\n| --- | --- |");
    for _ in &selected {
        out.push_str(" --- |");
    }
    out.push_str(" --- |\n");
    for action in all().filter(|a| selected.iter().any(|s| a.keys_for(*s).is_some())) {
        out.push_str(&format!("| `{}` | {} |", action.scope.name(), action.doc_label));
        for surface in &selected {
            let keys = action.keys_for(*surface).map(|keys| {
                keys.iter()
                    .map(|key| format!("`{}`", key.label()))
                    .collect::<Vec<_>>()
                    .join(", ")
            });
            out.push_str(&format!(" {} |", keys.as_deref().unwrap_or("—")));
        }
        out.push_str(&format!(" {} |\n", action.surface_note.unwrap_or("")));
    }
    out
}

pub(in crate::tui) fn help_text() -> &'static str {
    use std::sync::OnceLock;
    static NATIVE: OnceLock<String> = OnceLock::new();
    NATIVE.get_or_init(|| {
        let mut out = String::new();
        let mut scopes = Vec::new();
        for action in all() { if !scopes.contains(&action.scope) { scopes.push(action.scope); } }
        scopes.sort_by_key(|scope| match scope {
            Scope::Global=>0, Scope::Screen(Screen::Torrents)=>1,
            Scope::Screen(Screen::Search)=>2, Scope::Pane(Screen::Search,Pane::Query)=>3,
            Scope::Pane(Screen::Search,Pane::Results)=>4,Scope::Pane(Screen::Search,Pane::Browse)=>5,
            Scope::Modal(ModalKind::Details)=>6,Scope::Modal(ModalKind::Providers)=>6,Scope::Modal(ModalKind::Files)=>7,
            Scope::Screen(Screen::Rss)=>8,Scope::Pane(Screen::Rss,Pane::Feeds)=>9,
            Scope::Pane(Screen::Rss,Pane::Items)=>10,Scope::Screen(Screen::Shows)=>11,
            Scope::Pane(Screen::Shows,Pane::Shows)=>12,Scope::Pane(Screen::Shows,Pane::Releases)=>13,
            Scope::Modal(ModalKind::Settings)=>14,Scope::Prompt=>15,Scope::Modal(ModalKind::Help)=>16,_=>17,
        });
        for scope in scopes {
            let rows: Vec<_> = all().filter(|a|a.scope==scope).filter(|a| a.surfaces.contains(&Surface::Tui)).collect();
            if rows.is_empty() { continue; }
            if !out.is_empty() { out.push('\n'); }
            out.push_str(&scope.name().to_uppercase());
            out.push('\n');
            for action in rows {
            let keys = action.keys.iter().map(|k| k.label()).collect::<Vec<_>>().join("/");
            out.push_str(&format!("  ({keys}) {}\n", action.doc_label));
        }
            }
        out.push_str("\nUppercase letters require Shift.\nText inputs accept typing; Esc leaves the input.\nRemoval requires lowercase y; every other key cancels.\nMouse: click selects; double-click activates; wheel scrolls.\nSelecting and hovering never change read state.\nNative streaming priority is automatic; files support include/skip.\n");
        out
    })
}

pub(in crate::tui) fn help_controls() -> &'static str {
    static CONTROLS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CONTROLS.get_or_init(|| {
        ["help.page_up", "help.page_down", "help.close"]
            .into_iter()
            .map(|id| {
                let action = all().find(|action| action.id == id).unwrap();
                let keys = action.keys.iter().map(|key| key.label()).collect::<Vec<_>>().join("/");
                format!("({keys}) {}", action.doc_label)
            })
            .collect::<Vec<_>>()
            .join(" · ")
    })
}
