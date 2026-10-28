//! Build an index of the given roots and time some queries.
//! `cargo run --release -p ox-core --example index_bench -- /`
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::Instant;

use ox_core::index::{FileIndex, IndexConfig, IndexQuery};
use ox_core::search::Query;

fn main() {
    let roots = std::env::args().skip(1).map(Into::into).collect();
    let cfg = IndexConfig {
        roots,
        exclude: vec!["proc".into(), "sys".into()],
        threads: 0,
    };
    let t = Instant::now();
    let idx = FileIndex::build(&cfg, &AtomicBool::new(false), &AtomicU64::new(0));
    println!(
        "built {} entries in {:.2?}, ~{} MB",
        idx.len(),
        t.elapsed(),
        idx.memory_bytes() / 1_000_000
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
