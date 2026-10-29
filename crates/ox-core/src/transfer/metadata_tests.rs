use super::*;
use crate::transfer::{
    ConflictPolicy, EngineConfig, JobState, TransferEngine, TransferKind, TransferRequest,
};
use std::time::{Duration, Instant, SystemTime};

fn copy_with(cfg: EngineConfig, src: &Path, dest: &Path) -> std::sync::Arc<Job> {
    let engine = TransferEngine::new(EngineConfig {
        buffer_size: 64 * 1024,
        ..cfg
    });
    let job = engine.submit(TransferRequest::new(
        TransferKind::Copy,
        vec![src.to_path_buf()],
        dest.to_path_buf(),
        ConflictPolicy::Ask,
    ));
    let start = Instant::now();
    while !job.state().is_finished() {
        assert!(start.elapsed() < Duration::from_secs(20), "job timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(job.state(), JobState::Completed);
    job
}

fn modified(p: &Path) -> SystemTime {
    fs::metadata(p).unwrap().modified().unwrap()
}

fn set_modified(p: &Path, t: SystemTime) {
    open_for_times(p)
        .unwrap()
        .set_times(FileTimes::new().set_modified(t))
        .unwrap();
}

#[test]
fn folder_dates_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    fs::create_dir_all(src.join("sub/deeper")).unwrap();
    fs::write(src.join("sub/deeper/f.txt"), "f").unwrap();
    fs::write(src.join("top.txt"), "t").unwrap();
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    for (i, dir) in ["sub/deeper", "sub", ""].iter().enumerate() {
        set_modified(&src.join(dir), old + Duration::from_secs(i as u64 * 60));
    }

    let dest = tmp.path().join("dest");
    fs::create_dir(&dest).unwrap();
    let job = copy_with(EngineConfig::default(), &src, &dest);
    assert!(job.errors().is_empty(), "{:?}", job.errors());
    for dir in ["sub/deeper", "sub", ""] {
        assert_eq!(
            modified(&dest.join("src").join(dir)),
            modified(&src.join(dir)),
            "{dir}"
        );
    }

    let plain = tmp.path().join("plain");
    fs::create_dir(&plain).unwrap();
    copy_with(
        EngineConfig {
            keep_folder_dates: false,
            ..EngineConfig::default()
        },
        &src,
        &plain,
    );
    assert!(modified(&plain.join("src/sub")) > old + Duration::from_secs(3600));
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorControl, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED,
    };

    fn stream(path: &Path, name: &str) -> PathBuf {
        let mut s = path.as_os_str().to_owned();
        s.push(format!(":{name}"));
        PathBuf::from(s)
    }

    fn streams(path: &Path) -> Vec<String> {
        let mut names = imp::named_streams(path).unwrap();
        names.sort();
        names
    }

    #[test]
    fn alternate_data_streams_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir(&src).unwrap();
        let file = src.join("download.exe");
        fs::write(&file, "MZ").unwrap();
        fs::write(stream(&file, "extra"), "more data").unwrap();
        fs::write(
            stream(&file, "Zone.Identifier"),
            "[ZoneTransfer]\r\nZoneId=3\r\n",
        )
        .unwrap();
        fs::write(stream(&src, "folder"), "folder data").unwrap();
        assert_eq!(
            streams(&file),
            [":Zone.Identifier:$DATA", ":extra:$DATA"],
            "test setup"
        );

        let keep = tmp.path().join("keep");
        fs::create_dir(&keep).unwrap();
        copy_with(
            EngineConfig {
                keep_streams: true,
                ..EngineConfig::default()
            },
            &src,
            &keep,
        );
        let copy = keep.join("src/download.exe");
        assert_eq!(streams(&copy), [":Zone.Identifier:$DATA", ":extra:$DATA"]);
        assert_eq!(
            fs::read_to_string(stream(&copy, "extra")).unwrap(),
            "more data"
        );
        assert_eq!(
            fs::read_to_string(stream(&keep.join("src"), "folder")).unwrap(),
            "folder data"
        );

        // Off: only the internet mark stays, and the copy keeps its date.
        let plain = tmp.path().join("plain");
        fs::create_dir(&plain).unwrap();
        copy_with(EngineConfig::default(), &src, &plain);
        let copy = plain.join("src/download.exe");
        assert_eq!(streams(&copy), [":Zone.Identifier:$DATA"]);
        assert_eq!(fs::read_to_string(&copy).unwrap(), "MZ");
        assert_eq!(modified(&copy), modified(&file));
        assert!(streams(&plain.join("src")).is_empty());
    }

    /// Whether the file has its own (not inherited) permissions, and how many entries.
    fn dacl(path: &Path) -> (bool, u16) {
        let w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut acl: *mut ACL = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: test-only call with valid out pointers.
        let err = unsafe {
            GetNamedSecurityInfoW(
                w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                null_mut(),
                null_mut(),
                &mut acl,
                null_mut(),
                &mut sd,
            )
        };
        assert_eq!(err, 0);
        let (mut control, mut rev) = (0u16, 0u32);
        // SAFETY: `sd` is valid; it is freed below.
        let count = unsafe {
            GetSecurityDescriptorControl(sd, &mut control, &mut rev);
            let count = (*acl).AceCount;
            windows_sys::Win32::Foundation::LocalFree(sd);
            count
        };
        (control & SE_DACL_PROTECTED != 0, count)
    }

    use std::os::windows::ffi::OsStrExt;

    #[test]
    fn permissions_are_kept() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir(&src).unwrap();
        let file = src.join("secret.txt");
        fs::write(&file, "s").unwrap();
        // Own permissions only: Everyone may do everything (so cleanup works).
        let ok = std::process::Command::new("icacls")
            .arg(&file)
            .args(["/inheritance:r", "/grant:r", "*S-1-1-0:(F)"])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "icacls failed");
        assert_eq!(dacl(&file), (true, 1), "test setup");

        let keep = tmp.path().join("keep");
        fs::create_dir(&keep).unwrap();
        let job = copy_with(
            EngineConfig {
                keep_permissions: true,
                ..EngineConfig::default()
            },
            &src,
            &keep,
        );
        assert!(job.errors().is_empty(), "{:?}", job.errors());
        assert_eq!(dacl(&keep.join("src/secret.txt")), (true, 1));
        // The folder had inherited permissions only: it still inherits.
        assert!(!dacl(&keep.join("src")).0);

        let plain = tmp.path().join("plain");
        fs::create_dir(&plain).unwrap();
        copy_with(EngineConfig::default(), &src, &plain);
        assert!(!dacl(&plain.join("src/secret.txt")).0);
    }
}
