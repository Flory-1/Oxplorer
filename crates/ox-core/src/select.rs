//! Selection helpers: select by pattern (`*.jpg; *.png; report*`) and
//! select all items of the same type.

use crate::entry::FileEntry;

/// Name patterns typed by the user, separated by `;` or `,`. `*` matches
/// any run of characters and `?` one character; a pattern without either
/// matches names that contain it (like the filter box). Case doesn't matter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Patterns(Vec<Vec<char>>);

impl Patterns {
    pub fn parse(text: &str) -> Self {
        Patterns(
            text.split([';', ',', '|'])
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(|p| {
                    let p = p.to_lowercase();
                    if p.contains(['*', '?']) {
                        p.chars().collect()
                    } else {
                        format!("*{p}*").chars().collect()
                    }
                })
                .collect(),
        )
    }

    /// Nothing usable was typed.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Does `name` (with its extension) match one of the patterns?
    pub fn matches(&self, name: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let name: Vec<char> = name.to_lowercase().chars().collect();
        self.0.iter().any(|p| crate::search::wildcard(p, &name))
    }
}

/// What "the same type" means: folders are one type, files go by extension.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKey {
    Folder,
    /// Lower-case extension ("" for files without one).
    Extension(String),
}

impl TypeKey {
    pub fn of(e: &FileEntry) -> Self {
        if e.is_dir {
            TypeKey::Folder
        } else {
            TypeKey::Extension(e.extension())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_match_like_explorer() {
        let p = Patterns::parse("*.jpg; *.PNG ,report*");
        assert!(p.matches("Holiday.JPG"));
        assert!(p.matches("x.png"));
        assert!(p.matches("Report 2024.docx"));
        assert!(!p.matches("x.jpeg"));
        assert!(!p.matches("my report.txt"));
        // `?` is one character.
        let p = Patterns::parse("img_??.*");
        assert!(p.matches("IMG_01.raw"));
        assert!(!p.matches("img_001.raw"));
    }

    #[test]
    fn plain_words_match_inside_names() {
        let p = Patterns::parse("  draft ");
        assert!(p.matches("Letter (Draft).txt"));
        assert!(!p.matches("final.txt"));
        assert_eq!(Patterns::parse(".tmp"), Patterns::parse("*.tmp*"));
    }

    #[test]
    fn empty_patterns_match_nothing() {
        for text in ["", " ; , ", "|"] {
            let p = Patterns::parse(text);
            assert!(p.is_empty(), "{text:?}");
            assert!(!p.matches("anything"));
        }
        assert!(!Patterns::parse("*").is_empty());
        assert!(Patterns::parse("*").matches("anything"));
    }

    #[test]
    fn types_are_folders_or_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let file = |name: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, b"").unwrap();
            FileEntry::from_path(&p).unwrap()
        };
        let sub = dir.path().join("sub.jpg");
        std::fs::create_dir(&sub).unwrap();
        let folder = FileEntry::from_path(&sub).unwrap();
        assert_eq!(TypeKey::of(&folder), TypeKey::Folder);
        assert_eq!(TypeKey::of(&file("a.JPG")), TypeKey::of(&file("b.jpg")));
        assert_ne!(TypeKey::of(&file("a.jpg")), TypeKey::of(&file("a.png")));
        assert_eq!(
            TypeKey::of(&file("README")),
            TypeKey::Extension(String::new())
        );
    }
}
