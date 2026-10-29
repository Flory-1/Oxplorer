use super::*;

#[test]
fn plain_and_resolved_targets() {
    assert_eq!(plain(Path::new(r"\\?\C:\Data")), PathBuf::from(r"C:\Data"));
    assert_eq!(
        plain(Path::new(r"\\?\UNC\server\share")),
        PathBuf::from(r"\\server\share")
    );
    assert_eq!(plain(Path::new(r"\??\D:\x")), PathBuf::from(r"D:\x"));
    if cfg!(windows) {
        assert_eq!(
            resolve(Path::new(r"C:\a\b\link"), Path::new(r"..\c\file.txt")),
            PathBuf::from(r"C:\a\c\file.txt")
        );
        assert_eq!(
            resolve(Path::new(r"C:\a\link"), Path::new(r"\\?\D:\t")),
            PathBuf::from(r"D:\t")
        );
    }
}

#[test]
fn checks_before_creating() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::write(p.join("file.txt"), "x").unwrap();
    fs::create_dir(p.join("folder")).unwrap();
    // A junction to a file, a hard link to a folder, a missing target, a taken name.
    assert!(create(LinkKind::Junction, &p.join("j"), &p.join("file.txt")).is_err());
    assert!(create(LinkKind::Hard, &p.join("h"), &p.join("folder")).is_err());
    assert!(create(LinkKind::Hard, &p.join("h"), &p.join("gone.txt")).is_err());
    assert!(create(LinkKind::Hard, &p.join("folder"), &p.join("file.txt")).is_err());
    assert!(!fsops::exists(&p.join("j")) && !fsops::exists(&p.join("h")));
}

#[test]
fn hard_link_is_removed_without_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let target = p.join("data.txt");
    fs::write(&target, "hello").unwrap();
    let link = p.join("again.txt");
    create(LinkKind::Hard, &link, &target).unwrap();
    assert_eq!(fs::read_to_string(&link).unwrap(), "hello");
    // Hard links are plain files: no link info.
    assert!(info(&link).is_none());
    assert!(is_link(&link, LinkKind::Hard, &target));
    remove(&link, LinkKind::Hard).unwrap();
    assert!(!fsops::exists(&link));
    assert_eq!(fs::read_to_string(&target).unwrap(), "hello");
    // The last name of a file isn't a removable link.
    fs::hard_link(&target, &link).unwrap();
    fs::remove_file(&target).unwrap();
    assert!(!is_link(&link, LinkKind::Hard, &target));
}

#[test]
fn symbolic_link_to_a_file_or_why_not() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let target = p.join("data.txt");
    fs::write(&target, "hello").unwrap();
    let link = p.join("link.txt");
    match create(LinkKind::Symbolic, &link, &target) {
        Ok(()) => {
            let i = info(&link).unwrap();
            assert_eq!(i.kind, LinkKind::Symbolic);
            assert_eq!(i.resolved(&link), target);
            assert!(is_link(&link, LinkKind::Symbolic, &target));
            remove(&link, LinkKind::Symbolic).unwrap();
            assert!(target.exists() && !fsops::exists(&link));
        }
        // Developer Mode is off: the message says so.
        Err(e) => assert!(e.contains("Developer Mode"), "{e}"),
    }
}

#[test]
fn link_into_a_folder_uses_the_targets_name() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let target = p.join("notes.txt");
    fs::write(&target, "x").unwrap();
    fs::create_dir(p.join("dest")).unwrap();
    let made = link_into(&target, &p.join("dest")).unwrap();
    assert_eq!(made.link, p.join("dest").join("notes.txt"));
    // A symbolic link, or a hard link where Windows doesn't allow those.
    assert_eq!(made.fallback, made.kind == LinkKind::Hard);
    assert!(is_link(&made.link, made.kind, &target));
    // Again: a unique name.
    let again = link_into(&target, &p.join("dest")).unwrap();
    assert_eq!(again.link, p.join("dest").join("notes (2).txt"));
}

#[cfg(windows)]
#[test]
fn junction_points_to_its_folder_and_is_removed_alone() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let target = p.join("real");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("inside.txt"), "keep me").unwrap();
    let link = p.join("jump");
    create(LinkKind::Junction, &link, &target).unwrap();

    assert_eq!(
        fs::read_to_string(link.join("inside.txt")).unwrap(),
        "keep me"
    );
    let i = info(&link).unwrap();
    assert_eq!(i.kind, LinkKind::Junction);
    assert_eq!(i.resolved(&link), target);
    assert!(is_link(&link, LinkKind::Junction, &target));
    // The file view sees a link to a folder.
    let e = crate::entry::FileEntry::from_path(&link).unwrap();
    assert!(e.is_symlink && e.is_dir);

    remove(&link, LinkKind::Junction).unwrap();
    assert!(!fsops::exists(&link));
    assert_eq!(
        fs::read_to_string(target.join("inside.txt")).unwrap(),
        "keep me"
    );
}

#[cfg(windows)]
#[test]
fn folders_are_linked_with_junctions() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir_all(p.join("src/sub")).unwrap();
    fs::create_dir(p.join("dest")).unwrap();
    let made = link_into(&p.join("src"), &p.join("dest")).unwrap();
    assert_eq!(made.kind, LinkKind::Junction);
    assert!(!made.fallback);
    assert!(made.link.join("sub").is_dir());
}

#[cfg(windows)]
#[test]
fn junction_buffer_layout() {
    let buf = imp::mount_point_buffer(Path::new(r"C:\Data\")).unwrap();
    let u16_at = |i: usize| u16::from_le_bytes([buf[i], buf[i + 1]]);
    assert_eq!(&buf[..4], &0xA000_0003u32.to_le_bytes());
    let subst = r"\??\C:\Data".len() * 2;
    let print = r"C:\Data".len() * 2;
    assert_eq!(u16_at(4) as usize, 8 + subst + 2 + print + 2);
    assert_eq!(u16_at(8), 0);
    assert_eq!(u16_at(10) as usize, subst);
    assert_eq!(u16_at(12) as usize, subst + 2);
    assert_eq!(u16_at(14) as usize, print);
    assert_eq!(buf.len(), 16 + subst + 2 + print + 2);
    // A drive's root keeps its backslash.
    let root = imp::mount_point_buffer(Path::new(r"D:\")).unwrap();
    assert_eq!(
        u16::from_le_bytes([root[10], root[11]]) as usize,
        r"\??\D:\".len() * 2
    );
}

#[cfg(windows)]
#[test]
fn flat_view_does_not_follow_junctions() {
    use crate::flat::{self, FlatEvent, FlatOptions};
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir(p.join("a")).unwrap();
    fs::write(p.join("a/file.txt"), "x").unwrap();
    // A loop back to the top.
    create(LinkKind::Junction, &p.join("a/loop"), p).unwrap();
    let handle = flat::start(p.to_path_buf(), FlatOptions::default(), || {});
    let mut files = Vec::new();
    for ev in handle.events.iter() {
        match ev {
            FlatEvent::Files(f) => files.extend(f),
            FlatEvent::Done { .. } => break,
        }
    }
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, p.join("a/file.txt"));
}
