//! Numbers to read before listening: how far two renders that should sound alike differ, and how far the effects
//! stand out over the music.

use super::render::{PAL_HZ, SAMPLE_RATE};

/// One PAL frame's worth of samples.
fn frame() -> usize {
    (f64::from(SAMPLE_RATE) / PAL_HZ).round() as usize
}

/// RMS of each frame-long window from `from` seconds on.
fn levels(x: &[i16], from: f64) -> Vec<f64> {
    let w = frame();
    let start = (from * f64::from(SAMPLE_RATE)) as usize;
    x.get(start..)
        .unwrap_or(&[])
        .chunks_exact(w)
        .map(|c| (c.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / w as f64).sqrt())
        .collect()
}

/// `v` (a sample-scale RMS) in dB below full scale.
pub fn dbfs(v: f64) -> f64 {
    20.0 * (v.max(1e-3) / 32768.0).log10()
}

/// How far `b` differs from `a` over seconds 5–15, after shifting `b` by the lag (up to ±40 ms) that matches best:
/// the residual RMS as a share of `a`'s RMS, and the lag in milliseconds.
pub fn residual(a: &[i16], b: &[i16]) -> Option<(f64, f64)> {
    let rate = SAMPLE_RATE as usize;
    let (from, to) = (5 * rate, 15 * rate);
    if a.len() < to || b.len() < to + rate {
        return None;
    }
    let err = |lag: isize| {
        let (mut s, mut n) = (0.0, 0.0);
        for i in (from..to).step_by(7) {
            let j = i as isize + lag;
            if j < 0 {
                continue;
            }
            let d = f64::from(a[i]) - f64::from(b[j as usize]);
            s += d * d;
            n += 1.0;
        }
        (s / n).sqrt()
    };
    let span = (rate / 25) as isize;
    let coarse = (-span..=span)
        .step_by(48)
        .min_by(|&x, &y| err(x).total_cmp(&err(y)))?;
    let lag = (coarse - 48..=coarse + 48).min_by(|&x, &y| err(x).total_cmp(&err(y)))?;
    let signal = ((from..to)
        .step_by(7)
        .map(|i| f64::from(a[i]).powi(2))
        .sum::<f64>()
        / ((to - from) / 7) as f64)
        .sqrt();
    Some((
        err(lag) / signal.max(1.0),
        lag as f64 * 1000.0 / rate as f64,
    ))
}

/// The music's loud moments (90th percentile of frame levels) and the effects' median level, in dBFS, from three
/// renders after the first second: the music without voice 3, and voice 3 alone with and without effects. Effect
/// frames are those where the effects render is over 3 dB louder than the plain one and within 20 dB of its loudest
/// frames (the 99th percentile), so an effect's quiet tail and the oscillator's leak after it (#186) don't count.
pub fn effect_margin(music: &[i16], voice3_fx: &[i16], voice3: &[i16]) -> Option<(f64, f64)> {
    let mut m = levels(music, 1.0);
    m.sort_by(f64::total_cmp);
    let p90 = *m.get(m.len() * 9 / 10)?;
    let (a, b) = (levels(voice3_fx, 1.0), levels(voice3, 1.0));
    let mut sorted = a.clone();
    sorted.sort_by(f64::total_cmp);
    let floor = dbfs(*sorted.get(sorted.len() * 99 / 100)?) - 20.0;
    let mut fx: Vec<f64> = a
        .iter()
        .zip(&b)
        .filter(|(x, y)| dbfs(**x) - dbfs(**y) > 3.0 && dbfs(**x) >= floor)
        .map(|(x, _)| *x)
        .collect();
    fx.sort_by(f64::total_cmp);
    let median = *fx.get(fx.len() / 2)?;
    Some((dbfs(p90), dbfs(median)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(seconds: usize, amplitude: f64, period: usize) -> Vec<i16> {
        (0..seconds * SAMPLE_RATE as usize)
            .map(|i| {
                (amplitude * (std::f64::consts::TAU * (i % period) as f64 / period as f64).sin())
                    as i16
            })
            .collect()
    }

    #[test]
    fn a_delayed_copy_matches_at_its_lag() {
        let a = tone(20, 8000.0, 183);
        let mut b = vec![0i16; 480];
        b.extend_from_slice(&a);
        let (res, lag) = residual(&a, &b).unwrap();
        assert!(res < 0.01, "{res}");
        assert!((lag - 10.0).abs() < 0.1, "{lag}");
    }

    #[test]
    fn effects_are_found_where_voice_3_gets_louder() {
        let music = tone(5, 4000.0, 200);
        let quiet = vec![0i16; music.len()];
        let mut fx = quiet.clone();
        // An "effect" every second, 0.2 s long, at twice the music's amplitude, then a long faint tail (a leak
        // 40 dB down) that mustn't count.
        for s in 1..5 {
            let at = s * SAMPLE_RATE as usize;
            fx[at..at + 9600].copy_from_slice(&tone(1, 8000.0, 97)[..9600]);
            fx[at + 9600..at + 40_000].copy_from_slice(&tone(1, 80.0, 97)[..30_400]);
        }
        let (p90, median) = effect_margin(&music, &fx, &quiet).unwrap();
        assert!((median - p90 - 6.0).abs() < 0.5, "{p90} {median}");
    }
}
