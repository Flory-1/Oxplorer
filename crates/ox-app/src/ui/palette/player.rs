//! Palette entries for the built-in player: play the folder or the
//! selection, and play/pause, next, previous and stop while it plays.

use super::*;
use crate::app::PlayerAction;

pub(super) fn entries(app: &OxApp, v: &mut Vec<Entry>) {
    if !ox_core::player::supported() {
        return;
    }
    let mut cmd = |icon, label: &str, detail: String, run: Run| {
        v.push(Entry {
            icon,
            label: label.to_owned(),
            detail,
            run,
        })
    };
    let tab = app.tabs.active();
    if let Some(folder) = app
        .current_folder()
        .filter(|_| tab.location().shows_files())
    {
        cmd(
            "▶",
            tr("Play all in this folder"),
            folder.display().to_string(),
            act(move || Action::Player(PlayerAction::PlayFolder(folder))),
        );
    }
    let selection = tab.view.selected_paths();
    if !selection.is_empty() && !tab.location().is_bin() {
        cmd(
            "▶",
            tr("Play the selected items"),
            String::new(),
            act(move || Action::Player(PlayerAction::PlayAll(selection))),
        );
    }
    if app.player.playback.is_none() {
        return;
    }
    let player = |a: fn() -> PlayerAction| act(move || Action::Player(a()));
    cmd(
        "▶",
        tr("Player: play / pause"),
        shortcuts::label(Command::PlayerPlayPause),
        player(|| PlayerAction::PlayPause),
    );
    cmd(
        "⏭",
        tr("Player: next file"),
        shortcuts::label(Command::PlayerNext),
        player(|| PlayerAction::Next),
    );
    cmd(
        "⏮",
        tr("Player: previous file"),
        shortcuts::label(Command::PlayerPrevious),
        player(|| PlayerAction::Previous),
    );
    cmd(
        "🎵",
        tr("Show the playing file"),
        app.player.title(),
        player(|| PlayerAction::ShowCurrent),
    );
    cmd(
        "⏹",
        tr("Stop the player"),
        String::new(),
        player(|| PlayerAction::Close),
    );
}
