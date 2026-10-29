//! End to end: the service binary in console mode, talked to through its
//! named pipe. Needs administrator rights (it opens the volumes) and must
//! not run next to an installed, running service:
//! `cargo test -p ox-indexer -- --ignored` in an elevated terminal.
#![cfg(windows)]

use std::io::Write;
use std::process::{Child, Command};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use ox_core::index::helper::client::ClientError;
use ox_core::index::helper::connect_running;
use ox_core::index::helper::proto::ErrorCode;

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore]
fn console_mode_serves_the_volumes() {
    let _service = Running(
        Command::new(env!("CARGO_BIN_EXE_oxplorer-indexer"))
            .arg("--console")
            .spawn()
            .unwrap(),
    );
    let started = Instant::now();
    let mut client = loop {
        match connect_running() {
            Ok(c) => break c,
            Err(_) if started.elapsed() < Duration::from_secs(10) => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => panic!("no pipe (error {e})"),
        }
    };

    let volumes = client.volumes().unwrap();
    assert!(volumes.contains(&'C'), "{volumes:?}");

    let (mut count, mut windows) = (0u64, 0);
    let total = client
        .enumerate('C', &AtomicBool::new(false), |r| {
            count += 1;
            if r.name.eq_ignore_ascii_case("Windows") && r.attrs & 0x10 != 0 {
                windows += 1;
            }
        })
        .unwrap();
    assert_eq!(total, count);
    assert!(count > 10_000, "only {count} records");
    assert!(windows >= 1);

    let j = client.query_journal('C').unwrap();
    assert!(j.first <= j.next);
    // Something happens on C: meanwhile; reading from a moment ago works.
    let mut f = tempfile_in_temp();
    f.write_all(b"x").unwrap();
    let (next, _) = client.read_journal('C', j.journal, j.next).unwrap();
    assert!(next >= j.next);
    assert!(matches!(
        client.read_journal('C', j.journal ^ 1, j.next),
        Err(ClientError::Service(ErrorCode::JournalGone))
    ));

    // A letter that isn't a fixed NTFS drive.
    if let Some(other) = ('D'..='Z').find(|d| !volumes.contains(d)) {
        assert!(matches!(
            client.query_journal(other),
            Err(ClientError::Service(ErrorCode::NotServed))
        ));
    }
    // Still serving after the errors.
    assert_eq!(client.volumes().unwrap(), volumes);
}

fn tempfile_in_temp() -> std::fs::File {
    let path = std::env::temp_dir().join(format!("ox-indexer-e2e-{}.txt", std::process::id()));
    std::fs::File::create(path).unwrap()
}
