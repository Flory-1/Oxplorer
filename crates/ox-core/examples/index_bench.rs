//! Build an index of the given roots and time some queries.
//! `cargo run --release -p ox-core --example index_bench -- /`
//! With `--fast` first, whole NTFS drives are read from the master file table
//! (Windows, as administrator), e.g. `… -- --fast C:\`.
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::Instant;

use ox_core::index::{FileIndex, IndexConfig, IndexQuery};
use ox_core::search::Query;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fast = args.first().is_some_and(|a| a == "--fast");
    if fast {
        args.remove(0);
    }
    let roots = args.into_iter().map(Into::into).collect();
    let cfg = IndexConfig {
        roots,
        exclude: vec!["proc".into(), "sys".into()],
        threads: 0,
    };
    let t = Instant::now();
    let (cancel, progress) = (AtomicBool::new(false), AtomicU64::new(0));
    let idx = if fast {
        FileIndex::build_fast(&cfg, &cancel, &progress)
    } else {
        FileIndex::build(&cfg, &cancel, &progress)
    };
    println!(
        "built {} entries in {:.2?}, ~{} MB ({} roots from the MFT)",
        idx.len(),
        t.elapsed(),
        idx.memory_bytes() / 1_000_000,
        idx.mft_roots()
    );

    let file = std::env::temp_dir().join("oxplorer-bench-index.bin");
    let t = Instant::now();
    idx.save(&file).unwrap();
    println!(
        "saved in {:.2?} ({} MB)",
        t.elapsed(),
        std::fs::metadata(&file).unwrap().len() / 1_000_000
    );
    let t = Instant::now();
    let idx = FileIndex::load(&file).unwrap();
    println!("loaded in {:.2?}", t.elapsed());
    let _ = std::fs::remove_file(&file);

    let opts = IndexQuery {
        include_hidden: true,
        max_results: 50_000,
        ..Default::default()
    };
    for q in [
        "config",
        "*.rs",
        "lib",
        "kind:image",
        "/^mod\\.rs$/",
        "size:>10mb",
        "zzzznotthere",
    ] {
        let query = Query::parse(q, false);
        let t = Instant::now();
        let (ids, truncated) = idx.search(&query, &opts);
        let took = t.elapsed();
        println!(
            "{q:>16}: {:>6} hits{} in {took:.2?}",
            ids.len(),
            if truncated { "+" } else { "" }
        );
    }
}
