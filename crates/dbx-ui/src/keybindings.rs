//! Zed-style contextual user keymap, applied after the application's defaults.
use gpui::{App, Global, KeyBinding, KeyBindingContextPredicate};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, rc::Rc};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct KeymapBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub bindings: BTreeMap<String, Option<String>>,
}
#[derive(Clone)]
pub struct KeymapState {
    defaults: Vec<KeyBinding>,
    pub document: String,
    pub load_error: Option<String>,
}
impl Global for KeymapState {}

pub fn path() -> PathBuf {
    crate::config::directory()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("keymap.json")
}
pub fn initial_document() -> String {
    "[\n  {\n    \"context\": \"QueryWorkbench\",\n    \"bindings\": {}\n  }\n]\n".into()
}
pub fn parse(document: &str, cx: &App) -> Result<Vec<KeyBinding>, String> {
    let blocks: Vec<KeymapBlock> =
        serde_json::from_str(document).map_err(|error| error.to_string())?;
    let mut bindings = Vec::new();
    for block in blocks {
        let predicate = block
            .context
            .as_deref()
            .map(KeyBindingContextPredicate::parse)
            .transpose()
            .map_err(|error| error.to_string())?
            .map(Rc::new);
        for (keys, action) in block.bindings {
            if keys.trim().is_empty() {
                return Err("A shortcut cannot be empty".into());
            }
            let action = match action {
                Some(name) => cx
                    .build_action(&name, None)
                    .map_err(|error| error.to_string())?,
                None => Box::new(gpui::NoAction),
            };
            bindings.push(
                KeyBinding::load(
                    &keys,
                    action,
                    predicate.clone(),
                    false,
                    None,
                    &gpui::DummyKeyboardMapper,
                )
                .map_err(|error| error.to_string())?,
            );
        }
    }
    Ok(bindings)
}
pub fn init(cx: &mut App) {
    let defaults = cx.key_bindings().borrow().bindings().cloned().collect();
    let document = std::fs::read_to_string(path()).unwrap_or_else(|_| initial_document());
    let loaded = parse(&document, cx);
    let load_error = loaded.as_ref().err().cloned();
    cx.set_global(KeymapState {
        defaults,
        document,
        load_error,
    });
    if let Ok(bindings) = loaded {
        cx.bind_keys(bindings);
    }
}
pub fn save(document: String, cx: &mut App) -> Result<(), String> {
    // Validate everything before changing either the file or live bindings.
    let bindings = parse(&document, cx)?;
    let destination = path();
    let parent = destination.parent().ok_or("Keymap directory unavailable")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = parent.join(format!(".keymap-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, &document).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, destination).map_err(|error| error.to_string())?;
    let defaults = cx.global::<KeymapState>().defaults.clone();
    cx.clear_key_bindings();
    cx.bind_keys(defaults);
    cx.bind_keys(bindings);
    let state = cx.global_mut::<KeymapState>();
    state.document = document;
    state.load_error = None;
    Ok(())
}

pub fn label(name: &str) -> String {
    let name = name.rsplit("::").next().unwrap_or(name);
    let chars = name.chars().collect::<Vec<_>>();
    let mut label = String::new();
    for (index, character) in chars.iter().enumerate() {
        if index > 0
            && character.is_uppercase()
            && (chars[index - 1].is_lowercase()
                || chars.get(index + 1).is_some_and(|next| next.is_lowercase()))
        {
            label.push(' ');
        }
        label.push(*character);
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    fn validates_actions_contexts_and_keystrokes_before_applying(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            assert!(parse(r#"[{"context":"QueryWorkbench","bindings":{"ctrl-k ctrl-r":"dbx_ui::RunQuery","cmd-enter":null}}]"#, cx).is_ok());
            assert!(parse(r#"[{"bindings":{"cmd-k":"missing::Action"}}]"#, cx).is_err());
            assert!(parse(r#"[{"bindings":{},"contxt":"typo"}]"#, cx).is_err());
            assert!(parse(r#"[{"context":"(","bindings":{"cmd-k":null}}]"#, cx).is_err());
            assert!(parse(r#"[{"bindings":{"":null}}]"#, cx).is_err());
        });
    }
}
