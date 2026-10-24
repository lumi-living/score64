//! How long `monophonise_with` takes with drops, keeps and trims in force (#217), for `docs/TAD.md` §3.2. Run it in
//! release: `cargo test --release -p score64-core --test monophonise_bench -- --ignored --nocapture`. `BENCH_ONLY`
//! picks the cases whose label starts with it, `BENCH_MINS` the `min_rows` (default `1,2,4`).

use std::num::{NonZeroU8, NonZeroU32};
use std::time::Instant;

use score64_core::provenance::{
    Confidence, ContentHash, Decision, DecisionId, DropReason, Proposal, Provenance,
};
use score64_core::reduce::{MonoRules, Placement, monophonise_with};
use score64_core::score::{Attack, Note, Pitch, SubtuneName, Voice};

/// A linear congruential generator, so every run times the same case.
struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % n
    }
}

/// `notes` notes on random rows below `rows`, 1 to 6 rows long, at any confidence, a quarter of them tied; then
/// `drops`, `keeps` and `trims` of random notes, half a person's and half a model's.
fn case(
    seed: u64,
    notes: usize,
    rows: u32,
    drops: usize,
    keeps: usize,
    trims: usize,
) -> (Vec<Note>, Vec<Decision>) {
    let mut r = Lcg(seed);
    let voice = Voice::new(1).unwrap();
    let subtune = SubtuneName::new("main").unwrap();
    let soup: Vec<Note> = (0..notes)
        .map(|_| Note {
            row: r.below(u64::from(rows)) as u32,
            length: NonZeroU32::new(1 + r.below(6) as u32).unwrap(),
            pitch: Pitch::new(40 + r.below(24) as u8).unwrap(),
            attack: if r.below(4) == 0 {
                Attack::Legato
            } else {
                Attack::Fresh
            },
            instrument: None,
            source: None,
            confidence: Confidence::new(r.below(101) as f32 / 100.0).unwrap(),
            decision: None,
        })
        .collect();
    let mut decisions = Vec::new();
    let mut add = |proposal: Proposal, r: &mut Lcg| {
        let row = match proposal {
            Proposal::DropNote { row, .. }
            | Proposal::TrimNote { row, .. }
            | Proposal::KeepNote { row, .. } => row,
            _ => 0,
        };
        let provenance = if r.below(2) == 0 {
            Provenance::Manual
        } else {
            Provenance::Llm {
                model: "m".into(),
                prompt: ContentHash([1; 32]),
            }
        };
        decisions.push(Decision {
            id: DecisionId::new(format!("d{}", decisions.len())).unwrap(),
            subtune: subtune.clone(),
            bar: row / 16,
            proposal,
            provenance,
            confidence: Confidence::CERTAIN,
            reviewed: false,
        });
    };
    for _ in 0..drops {
        let n = &soup[r.below(notes as u64) as usize];
        let p = Proposal::DropNote {
            voice,
            row: n.row,
            pitch: n.pitch,
            length: n.length,
            source: None,
            reason: DropReason::Overlap,
        };
        add(p, &mut r);
    }
    for _ in 0..keeps {
        let n = &soup[r.below(notes as u64) as usize];
        let p = Proposal::KeepNote {
            voice,
            row: n.row,
            pitch: n.pitch,
        };
        add(p, &mut r);
    }
    for _ in 0..trims {
        let n = &soup[r.below(notes as u64) as usize];
        let length = NonZeroU32::new(1 + r.below(u64::from(n.length.get())) as u32).unwrap();
        let p = Proposal::TrimNote {
            voice,
            row: n.row,
            pitch: n.pitch,
            length,
            source: None,
        };
        add(p, &mut r);
    }
    (soup, decisions)
}

#[test]
#[ignore]
fn monophonise_with_timings() {
    let subtune = SubtuneName::new("main").unwrap();
    let at = Placement {
        subtune: &subtune,
        voice: Voice::new(1).unwrap(),
        rows_per_bar: NonZeroU8::new(16).unwrap(),
    };
    let only = std::env::var("BENCH_ONLY").ok();
    let mins: Vec<u32> = std::env::var("BENCH_MINS")
        .map(|s| s.split(',').map(|x| x.parse().unwrap()).collect())
        .unwrap_or(vec![1, 2, 4]);
    for (label, notes, rows, drops, keeps, trims) in [
        ("2k/4k", 2000usize, 4000u32, 1000usize, 500usize, 500usize),
        ("10k/10k", 10000, 10000, 5000, 2500, 2500),
        ("10k/2k", 10000, 2000, 5000, 2500, 2500),
        ("drops-only-10k/2k", 10000, 2000, 5000, 0, 2500),
        ("keeps-heavy-10k/2k", 10000, 2000, 1000, 5000, 1000),
    ] {
        if only.as_deref().is_some_and(|o| !label.starts_with(o)) {
            continue;
        }
        for &min in &mins {
            let (soup, decisions) = case(42 + u64::from(min), notes, rows, drops, keeps, trims);
            let rules = MonoRules {
                min_rows: NonZeroU32::new(min).unwrap(),
                ..MonoRules::default()
            };
            // Best of three.
            let mut best = f64::MAX;
            for _ in 0..3 {
                let t = Instant::now();
                let r = monophonise_with(soup.clone(), &at, &rules, &decisions);
                best = best.min(t.elapsed().as_secs_f64());
                std::hint::black_box(r);
            }
            eprintln!("{label} min_rows {min}: {:.1} ms", best * 1000.0);
        }
    }
}
