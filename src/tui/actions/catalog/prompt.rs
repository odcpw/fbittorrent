use super::*;
pub(super) const ACTIONS: &[Action] = &[
    action!(
        "prompt.confirm",
        Scope::Prompt,
        &[KeyChord::plain(KeyCode::Char('y'))],
        "Remove",
        10,
        |a| a.prompt.as_ref().is_some_and(|p| p.yes_no),
        Some(|a| a.on_prompt_action(Intent::Confirm)),
        TDP,
        None,
        None,
        None
    ),
    action!(
        "prompt.cancel",
        Scope::Prompt,
        &[KeyChord::plain(KeyCode::Escape)],
        "Cancel",
        10,
        |_| true,
        Some(|a| a.on_prompt_action(Intent::Cancel)),
        TDP,
        None,
        None,
        None
    ),
    action!(
        "prompt.submit",
        Scope::Prompt,
        &[KeyChord::plain(KeyCode::Enter)],
        "Submit",
        10,
        |a| a.prompt.as_ref().is_some_and(|p| !p.yes_no || matches!(p.kind, PromptKind::ConfirmSearchBatch { .. })),
        Some(|a| a.on_prompt_action(Intent::Primary)),
        TD,
        None,
        None,
        None
    ),
];
