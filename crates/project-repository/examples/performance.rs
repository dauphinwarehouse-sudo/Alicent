use alicent_domain::{DocumentKind, SaveDocument};
use alicent_project_repository::Repository;
use serde::Serialize;
use std::{
    env, fs,
    path::PathBuf,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use uuid::Uuid;

#[derive(Clone, Copy)]
struct Profile {
    name: &'static str,
    words: usize,
    nodes: usize,
    large_document_words: usize,
    samples: usize,
}

#[derive(Serialize)]
struct Report {
    profile: &'static str,
    words: usize,
    nodes: usize,
    large_document_words: usize,
    dataset_seconds: f64,
    database_bytes: u64,
    rss_kib: Option<u64>,
    open_ms: Percentiles,
    read_large_document_ms: Percentiles,
    search_common_ms: Percentiles,
    search_rare_ms: Percentiles,
}

#[derive(Serialize)]
struct Percentiles {
    p50: f64,
    p95: f64,
    max: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let profile_name = value_after(&args, "--profile").unwrap_or("smoke");
    let output = value_after(&args, "--output").map(PathBuf::from);
    let keep = args.iter().any(|arg| arg == "--keep");
    let profile = match profile_name {
        "smoke" => Profile {
            name: "smoke",
            words: 10_000,
            nodes: 200,
            large_document_words: 10_000,
            samples: 7,
        },
        "medium" => Profile {
            name: "medium",
            words: 500_000,
            nodes: 2_000,
            large_document_words: 200_000,
            samples: 15,
        },
        "acceptance" => Profile {
            name: "acceptance",
            words: 5_000_000,
            nodes: 20_000,
            large_document_words: 200_000,
            samples: 25,
        },
        other => return Err(format!("unknown profile: {other}").into()),
    };

    let temp = TempDir::new()?;
    let started = Instant::now();
    let mut repo = Repository::create(temp.path(), "Performance fixture")?;
    let root = repo.root().to_path_buf();
    let large = repo.create_document("Большой документ", DocumentKind::Scene, None)?;
    repo.save(SaveDocument {
        command_id: Uuid::new_v4(),
        document_id: large.summary.id,
        expected_revision: large.summary.revision,
        content: words(profile.large_document_words, "редкиймаркер"),
    })?;

    let remaining_words = profile.words.saturating_sub(profile.large_document_words);
    let content_nodes = profile.nodes.saturating_sub(1).max(1);
    let words_per_document = remaining_words.div_ceil(content_nodes);
    for index in 0..content_nodes {
        let doc = repo.create_document(&format!("Сцена {index:05}"), DocumentKind::Scene, None)?;
        let requested = remaining_words
            .saturating_sub(index * words_per_document)
            .min(words_per_document);
        if requested > 0 {
            repo.save(SaveDocument {
                command_id: Uuid::new_v4(),
                document_id: doc.summary.id,
                expected_revision: doc.summary.revision,
                content: words(requested, "общиймаркер"),
            })?;
        }
    }
    let dataset_seconds = started.elapsed().as_secs_f64();
    drop(repo);

    let open_ms = sample(profile.samples, || {
        let opened = Repository::open(&root)?;
        std::hint::black_box(opened.project()?);
        Ok(())
    })?;
    let repo = Repository::open(&root)?;
    let read_large_document_ms = sample(profile.samples, || {
        std::hint::black_box(repo.read(large.summary.id)?);
        Ok(())
    })?;
    let search_common_ms = sample(profile.samples, || {
        std::hint::black_box(repo.search("общиймаркер", 20)?);
        Ok(())
    })?;
    let search_rare_ms = sample(profile.samples, || {
        std::hint::black_box(repo.search("редкиймаркер", 20)?);
        Ok(())
    })?;
    let report = Report {
        profile: profile.name,
        words: profile.words,
        nodes: profile.nodes,
        large_document_words: profile.large_document_words,
        dataset_seconds,
        database_bytes: fs::metadata(root.join("project.sqlite3"))?.len(),
        rss_kib: resident_memory_kib(),
        open_ms,
        read_large_document_ms,
        search_common_ms,
        search_rare_ms,
    };
    let json = serde_json::to_string_pretty(&report)?;
    println!("{json}");
    if let Some(path) = output {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, format!("{json}\n"))?;
    }
    if keep {
        let kept = temp.keep();
        eprintln!("dataset kept at {}", kept.display());
    }
    Ok(())
}

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn words(count: usize, marker: &str) -> String {
    let vocabulary = [
        "Алиса",
        "шла",
        "через",
        "тихий",
        "город",
        "и",
        "слышала",
        "далёкий",
        "звон",
        marker,
    ];
    (0..count)
        .map(|index| vocabulary[index % vocabulary.len()])
        .collect::<Vec<_>>()
        .join(" ")
}

fn sample(
    count: usize,
    mut operation: impl FnMut() -> alicent_project_repository::Result<()>,
) -> alicent_project_repository::Result<Percentiles> {
    operation()?;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let started = Instant::now();
        operation()?;
        values.push(started.elapsed());
    }
    values.sort_unstable();
    Ok(Percentiles {
        p50: millis(values[percentile_index(values.len(), 0.50)]),
        p95: millis(values[percentile_index(values.len(), 0.95)]),
        max: millis(*values.last().expect("at least one sample")),
    })
}

fn percentile_index(len: usize, percentile: f64) -> usize {
    ((len as f64 * percentile).ceil() as usize)
        .saturating_sub(1)
        .min(len - 1)
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

#[cfg(target_os = "linux")]
fn resident_memory_kib() -> Option<u64> {
    fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(not(target_os = "linux"))]
fn resident_memory_kib() -> Option<u64> {
    None
}
