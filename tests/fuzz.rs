//! Mutation harness: truncated, byte-flipped and zeroed inputs must produce an
//! error, never a panic and never a hang. The corpus sweep additionally runs the
//! built binary over every sample so the CLI's exit status is covered too.

use std::path::PathBuf;

use one_note_converter::doc::{extract, AssetRef};
use one_note_converter::markdown;

/// Sample files: `$ONENOTE_SAMPLES`, then the local corpus used during
/// development, then the committed fixture.
fn samples() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(dir) = std::env::var("ONENOTE_SAMPLES") {
        push_dir(PathBuf::from(dir), &mut found);
    }
    for dir in ["/tmp/one-samples", "/tmp/corpus2/files"] {
        push_dir(PathBuf::from(dir), &mut found);
    }
    found.push(PathBuf::from("tests/fixtures/testOneNote2016.one"));
    found.retain(|path| path.is_file());
    found.sort();
    found
}

fn push_dir(dir: PathBuf, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "one") {
            into.push(path);
        }
    }
}

/// Small deterministic PRNG so failures reproduce exactly.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 11
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next() % bound as u64) as usize
        }
    }
}

fn mutate(original: &[u8], round: usize) -> Vec<u8> {
    let mut rng = Lcg(0x5DEE_CE66 ^ (round as u64) << 17);
    let mut data = original.to_vec();
    if data.is_empty() {
        return data;
    }
    match round % 5 {
        // Truncate somewhere after the header.
        0 => data.truncate(1024 + rng.below(data.len().saturating_sub(1024) + 1)),
        // Flip a handful of bytes anywhere.
        1 => {
            for _ in 0..1 + rng.below(8) {
                let at = rng.below(data.len());
                data[at] ^= 1 << rng.below(8);
            }
        }
        // Zero a run inside the body.
        2 => {
            let start = rng.below(data.len());
            let end = (start + rng.below(4096)).min(data.len());
            data[start..end].fill(0);
        }
        // Overwrite the header with another prefix of the same file.
        3 => {
            let len = rng.below(64.min(data.len()));
            let from = rng.below(data.len() - len.max(1));
            let prefix: Vec<u8> = data[from..from + len].to_vec();
            data[..len].copy_from_slice(&prefix);
        }
        // Splice an interior window over the start.
        _ => {
            let start = rng.below(data.len());
            let end = (start + 256).min(data.len());
            let window: Vec<u8> = data[start..end].to_vec();
            let at = rng.below(data.len().saturating_sub(window.len()).max(1));
            data[at..at + window.len()].copy_from_slice(&window);
        }
    }
    data
}

#[test]
fn mutated_inputs_error_without_panicking() {
    let mut stats = (0usize, 0usize);
    for file in samples().iter().take(4) {
        let original = std::fs::read(file).expect("sample readable");
        for round in 0..40 {
            let data = mutate(&original, round);
            // Any Err is fine; a panic escapes this call and fails the test.
            match one_note_converter::load(&data) {
                Ok((_header, store)) => {
                    stats.0 += 1;
                    let pages = extract(&store);
                    let mut resolver = |image: &AssetRef| image.filename.clone();
                    let _ = markdown::render(&pages, &mut resolver);
                }
                Err(_) => stats.1 += 1,
            }
        }
    }
    eprintln!("mutations: {} parsed, {} rejected", stats.0, stats.1);
    assert!(
        stats.1 > 0,
        "no mutation was rejected — harness is not exercising"
    );
}

#[test]
fn every_sample_exits_with_a_status() {
    let binary = env!("CARGO_BIN_EXE_onenote2md");
    let dir = std::env::temp_dir().join("onenote2md-fuzz-sweep");
    let _ = std::fs::create_dir_all(&dir);
    for file in samples() {
        let output = std::process::Command::new(binary)
            .arg("convert")
            .arg(&file)
            .arg("-o")
            .arg(dir.join("out.md"))
            .arg("--assets")
            .arg(dir.join("assets"))
            .output()
            .expect("binary runs");
        // 0 = converted, 1 = reported error. A signal death has no code.
        assert!(
            output.status.code().is_some(),
            "{} died by signal: {:?}",
            file.display(),
            output.status
        );
        assert!(
            output.status.code().is_some_and(|code| code <= 1),
            "{} exited with {:?}",
            file.display(),
            output.status
        );
    }
}
