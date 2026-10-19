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
        |a| a.prompt.as_ref().is_some_and(|p| !p.yes_no),
        Some(|a| a.on_prompt_action(Intent::Primary)),
        TD,
        None,
        None,
        None
    ),
    action!(
        "prompt.sequential",
        Scope::Prompt,
        &[KeyChord::plain(KeyCode::F(2))],
        "Sequential (legacy)",
        40,
        |a| a
            .prompt
            .as_ref()
            .is_some_and(|p| p.kind == PromptKind::AddMagnet && !p.native_streaming),
        Some(|a| a.on_prompt_action(Intent::Sequential)),
        T,
        None,
        None,
        None
    ),
    action!(
        "prompt.first_last",
        Scope::Prompt,
        &[KeyChord::plain(KeyCode::F(3))],
        "First/last (legacy)",
        40,
        |a| a
            .prompt
            .as_ref()
            .is_some_and(|p| p.kind == PromptKind::AddMagnet && !p.native_streaming),
        Some(|a| a.on_prompt_action(Intent::FirstLast)),
        T,
        None,
        None,
        None
    ),
    action!(
        "prompt.seed",
        Scope::Prompt,
        &[KeyChord::plain(KeyCode::F(4))],
        "Seed on/off",
        40,
        |a| a.prompt.as_ref().is_some_and(|p| p.kind == PromptKind::AddMagnet),
        Some(|a| a.on_prompt_action(Intent::Seed)),
        T,
        None,
        None,
        None
    ),
];
