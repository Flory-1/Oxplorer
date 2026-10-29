//! Every text the interface translates must have a German translation.
//!
//! Scans the sources for string literals passed to `tr(…)`, `t(…)`,
//! `i18n::t(…)` and `trf!(…)` and looks each one up in the German table.
//! Texts kept in tables and translated where they are shown (help topics,
//! settings sections, shortcut labels, …) are checked by their own modules.

/// An app source file, by its path from `src/`.
macro_rules! app {
    ($path:literal) => {
        ($path, include_str!($path))
    };
}

/// An `ox-core` source file, by its path from `ox-core/src/`.
macro_rules! core {
    ($path:literal) => {
        ($path, include_str!(concat!("../../ox-core/src/", $path)))
    };
}

pub(crate) const SOURCES: &[(&str, &str)] = &[
    app!("app.rs"),
    app!("app/action.rs"),
    app!("app/cleanup.rs"),
    app!("app/cloud.rs"),
    app!("app/compare.rs"),
    app!("app/flat.rs"),
    app!("app/input.rs"),
    app!("app/links.rs"),
    app!("app/instances.rs"),
    app!("app/library.rs"),
    app!("app/poll.rs"),
    app!("app/process.rs"),
    app!("app/properties.rs"),
    app!("app/sessions.rs"),
    app!("app/search_history.rs"),
    app!("app/quick_look.rs"),
    app!("app/select.rs"),
    app!("app/undo.rs"),
    app!("app/view.rs"),
    app!("app/transfer_watch.rs"),
    app!("app/window.rs"),
    app!("column_values.rs"),
    app!("tabs.rs"),
    app!("tabs/flat.rs"),
    app!("tabs/groups.rs"),
    app!("tabs/item_groups.rs"),
    app!("tabs/location.rs"),
    app!("tabs/session.rs"),
    app!("thumbs.rs"),
    app!("ui/cleanup.rs"),
    app!("ui/cloud.rs"),
    app!("ui/compare.rs"),
    app!("ui/dialogs.rs"),
    app!("ui/files.rs"),
    app!("ui/files/cells.rs"),
    app!("ui/files/column_header.rs"),
    app!("ui/files/details.rs"),
    app!("ui/files/flat.rs"),
    app!("ui/files/grid.rs"),
    app!("ui/files/grouped.rs"),
    app!("ui/files/item.rs"),
    app!("ui/files/zoom.rs"),
    app!("ui/header.rs"),
    app!("ui/header/caption.rs"),
    app!("ui/header/command_bar.rs"),
    app!("ui/header/nav.rs"),
    app!("ui/header/nav/suggest.rs"),
    app!("ui/header/tab_strip.rs"),
    app!("ui/help_page.rs"),
    app!("ui/home.rs"),
    app!("ui/link_dialog.rs"),
    app!("ui/links.rs"),
    app!("ui/menus.rs"),
    app!("ui/quick_look.rs"),
    app!("ui/select_pattern.rs"),
    app!("ui/mod.rs"),
    app!("ui/palette.rs"),
    app!("ui/palette/cleanup.rs"),
    app!("ui/palette/tabs.rs"),
    app!("ui/palette/views.rs"),
    app!("ui/preview.rs"),
    app!("ui/preview/font.rs"),
    app!("ui/preview/git_graph.rs"),
    app!("ui/preview/info.rs"),
    app!("ui/preview/large.rs"),
    app!("ui/preview/load.rs"),
    app!("ui/preview/player.rs"),
    app!("ui/settings_page.rs"),
    app!("ui/settings_page/keys.rs"),
    app!("ui/settings_page/search.rs"),
    app!("ui/settings_page/theme.rs"),
    app!("ui/settings_page/transfer.rs"),
    app!("ui/sessions.rs"),
    app!("ui/settings_page/views.rs"),
    app!("ui/sidebar.rs"),
    app!("ui/sidebar/libraries.rs"),
    app!("ui/transfers.rs"),
    app!("ui/transfers/ask.rs"),
    app!("ui/transfers/queue.rs"),
    app!("ui/transfers/when_done.rs"),
    app!("ui/transfers/resume.rs"),
    core!("archive.rs"),
    core!("columns.rs"),
    core!("cloud/mod.rs"),
    core!("default_app.rs"),
    core!("details.rs"),
    core!("entry.rs"),
    core!("folder_views.rs"),
    core!("fsops.rs"),
    core!("links.rs"),
    core!("grouping.rs"),
    core!("pathfmt.rs"),
    core!("places.rs"),
    core!("props.rs"),
    core!("player.rs"),
    core!("rename.rs"),
    core!("search_history/suggest.rs"),
    core!("tags.rs"),
    core!("textinfo.rs"),
    core!("transfer/mod.rs"),
    core!("transfer/plan.rs"),
    core!("undo.rs"),
    core!("transfer/when_done.rs"),
    core!("transfer/journal.rs"),
    core!("transfer/metadata.rs"),
];

/// The value of the string literal starting at `src[i]` (a `"`), and where it ends.
fn parse_literal(src: &str, i: usize) -> (String, usize) {
    let b = src.as_bytes();
    let mut out = String::new();
    let mut j = i + 1;
    loop {
        match b[j] {
            b'"' => return (out, j + 1),
            b'\\' => {
                j += 1;
                match b[j] {
                    b'\n' => {
                        // Line continuation: skip the newline and leading whitespace.
                        j += 1;
                        while b[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        continue;
                    }
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    b'r' => out.push('\r'),
                    b'0' => out.push('\0'),
                    b'\\' => out.push('\\'),
                    b'"' => out.push('"'),
                    b'\'' => out.push('\''),
                    b'x' => {
                        let code = u8::from_str_radix(&src[j + 1..j + 3], 16).unwrap();
                        out.push(code as char);
                        j += 2;
                    }
                    b'u' => {
                        let end = j + src[j..].find('}').unwrap();
                        let code = u32::from_str_radix(&src[j + 2..end], 16).unwrap();
                        out.push(char::from_u32(code).unwrap());
                        j = end;
                    }
                    other => panic!("unknown escape \\{}", other as char),
                }
                j += 1;
            }
            _ => {
                let c = src[j..].chars().next().unwrap();
                out.push(c);
                j += c.len_utf8();
            }
        }
    }
}

/// Literal arguments of translation calls (tests excluded).
fn translated_literals(src: &str) -> Vec<String> {
    // `include_str!` gives the file as checked out: with CRLF line endings on
    // Windows. rustc reads sources with LF only, so do the same.
    let src = src.replace("\r\n", "\n");
    let src = src.split("#[cfg(test)]").next().unwrap_or(&src);
    let mut found = Vec::new();
    for call in ["tr(", "t(", "trf!("] {
        let mut from = 0;
        while let Some(pos) = src[from..].find(call) {
            let at = from + pos;
            from = at + call.len();
            // Whole names only (`tr(` must not match `attr(`, `t(` not `tr(` etc.).
            let before = src[..at].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                continue;
            }
            // Skip comments.
            let line_start = src[..at].rfind('\n').map_or(0, |n| n + 1);
            if src[line_start..at].contains("//") {
                continue;
            }
            let rest = &src[from..];
            let skipped = rest.len() - rest.trim_start().len();
            if rest.trim_start().starts_with('"') {
                found.push(parse_literal(src, from + skipped).0);
            }
        }
    }
    found
}

#[test]
fn literals_are_parsed_like_rust_does() {
    let src = r#"tr("a \"b\" c\n\
                   d") trf!("{n} x", n = 1) attr("no") // tr("no")
                   t("\u{2192}")"#;
    assert_eq!(
        translated_literals(src),
        vec!["a \"b\" c\nd", "\u{2192}", "{n} x"]
    );
    // The same with Windows line endings (how Git checks files out there).
    assert_eq!(
        translated_literals(&src.replace('\n', "\r\n")),
        vec!["a \"b\" c\nd", "\u{2192}", "{n} x"]
    );
}

#[test]
fn every_translated_text_has_a_german_translation() {
    let mut missing = Vec::new();
    let mut count = 0;
    for (file, src) in SOURCES {
        for text in translated_literals(src) {
            count += 1;
            if ox_core::i18n::german(&text).is_none() {
                missing.push(format!("{file}: {text:?}"));
            }
        }
    }
    assert!(count > 400, "only {count} texts found: is the scan broken?");
    assert!(
        missing.is_empty(),
        "no German translation (add them to ox-core/src/i18n_de.rs):\n{}",
        missing.join("\n")
    );
}

#[test]
fn texts_translated_where_they_are_shown_have_german_too() {
    use ox_core::entry::FileKind;
    use ox_core::transfer::{ConflictPolicy, TransferKind};
    // Tests run in English, so these return the English text.
    let mut texts: Vec<&str> = crate::shortcuts::COMMANDS.iter().map(|c| c.label).collect();
    texts.extend(
        [
            FileKind::Folder,
            FileKind::Image,
            FileKind::Video,
            FileKind::Audio,
            FileKind::Document,
            FileKind::Spreadsheet,
            FileKind::Presentation,
            FileKind::Pdf,
            FileKind::Archive,
            FileKind::Code,
            FileKind::Text,
            FileKind::Executable,
            FileKind::Font,
            FileKind::Other,
        ]
        .map(FileKind::label),
    );
    texts.extend(ox_core::tags::TagColor::ALL.map(|c| c.name()));
    texts.extend(ox_core::columns::ColumnId::ALL.map(|c| c.label()));
    texts.extend(ox_core::entry::SortKey::ALL.map(|k| k.label()));
    texts.extend(ox_core::grouping::GroupBy::ALL.map(|g| g.label()));
    use ox_core::grouping::{DateBucket as D, SizeBucket as S};
    texts.extend(
        [
            D::Today,
            D::Yesterday,
            D::EarlierThisWeek,
            D::LastWeek,
            D::EarlierThisMonth,
            D::LastMonth,
            D::EarlierThisYear,
            D::LongAgo,
        ]
        .map(D::label),
    );
    texts.extend(
        [
            S::Empty,
            S::Tiny,
            S::Small,
            S::Medium,
            S::Large,
            S::Huge,
            S::Gigantic,
        ]
        .map(S::label),
    );
    texts.extend(crate::tabs::GROUP_COLORS.map(|(name, _)| name));
    texts.extend(ConflictPolicy::ALL.map(ConflictPolicy::label));
    texts.extend([TransferKind::Copy.verb(), TransferKind::Move.verb()]);
    {
        use ox_core::links::LinkKind;
        use ox_core::undo::Kind;
        texts.extend([LinkKind::Symbolic, LinkKind::Junction, LinkKind::Hard].map(LinkKind::label));
        texts.extend(
            [
                Kind::Rename,
                Kind::Move,
                Kind::Copy,
                Kind::NewFolder,
                Kind::NewFile,
                Kind::NewLink,
                Kind::Delete,
            ]
            .map(Kind::label),
        );
    }
    let new_items = ox_core::settings::NewItems::default();
    texts.extend(new_items.0.iter().map(|i| i.label.as_str()));
    for text in texts {
        assert!(
            ox_core::i18n::german(text).is_some(),
            "no German for {text:?}"
        );
    }
}
