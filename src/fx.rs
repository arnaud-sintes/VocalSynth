//! Post-synthesis DSP: silence trimming and a Freeverb-style reverb.

/// Removes leading/trailing silence. A frame counts as silence when its loudest
/// channel is below `threshold_db` dBFS. A short pad is kept on each side so the
/// attack/decay of speech isn't clipped. Returns the input unchanged if the whole
/// clip is below threshold.
pub fn trim(channels: Vec<Vec<f32>>, sample_rate: u32, threshold_db: f64, pad_ms: f64) -> Vec<Vec<f32>> {
    let len = channels.first().map_or(0, Vec::len);
    if len == 0 {
        return channels;
    }

    let thr = 10f64.powf(threshold_db / 20.0) as f32;
    let mut first = None;
    let mut last = 0;
    for i in 0..len {
        let m = channels.iter().fold(0f32, |m, c| m.max(c[i].abs()));
        if m > thr {
            first.get_or_insert(i);
            last = i;
        }
    }
    let Some(first) = first else {
        return channels; // pure silence — leave as-is
    };

    let pad = (pad_ms / 1000.0 * sample_rate as f64) as usize;
    let start = first.saturating_sub(pad);
    let end = (len - 1).min(last + pad);

    channels.iter().map(|c| c[start..=end].to_vec()).collect()
}

/// Freeverb-style reverb. `amount` is the wet mix (0..=1); the output is extended by
/// a decaying tail whose length depends on the room preset ("small" | "medium" |
/// "large"). Wet is level-matched to the dry peak so the mix is predictable, and the
/// result is peak-limited and faded to avoid clicks.
pub fn reverb(channels: Vec<Vec<f32>>, sample_rate: u32, amount: f64, room: &str) -> Vec<Vec<f32>> {
    let amount = amount.clamp(0.0, 1.0);
    let (room_size, damp, tail_sec) = room_preset(room);

    let in_len = channels.first().map_or(0, Vec::len);
    if in_len == 0 {
        return channels;
    }
    let out_len = in_len + (tail_sec * sample_rate as f64) as usize;

    const FIXED_GAIN: f32 = 0.015;
    let wet: Vec<Vec<f32>> = channels
        .iter()
        .map(|src| {
            let mut net = FreeverbNet::new(sample_rate, room_size, damp);
            (0..out_len)
                .map(|i| net.process(if i < in_len { src[i] * FIXED_GAIN } else { 0.0 }))
                .collect()
        })
        .collect();

    let dry_peak = peak(&channels);
    let wet_peak = peak(&wet);
    let w_scale = if wet_peak > 1e-9 { dry_peak / wet_peak } else { 0.0 };

    let dry_gain = (1.0 - 0.3 * amount) as f32;
    let wet_gain = amount as f32 * w_scale;
    let mut out: Vec<Vec<f32>> = channels
        .iter()
        .zip(&wet)
        .map(|(d, w)| {
            (0..out_len)
                .map(|i| {
                    let dry = if i < in_len { d[i] } else { 0.0 };
                    dry * dry_gain + w[i] * wet_gain
                })
                .collect()
        })
        .collect();

    let p = peak(&out);
    if p > 0.99 {
        scale_all(&mut out, 0.99 / p);
    }
    fade_out(&mut out, (0.02 * sample_rate as f64) as usize); // 20 ms fade so the tail end doesn't click
    out
}

/// (room size, damping, tail seconds)
fn room_preset(room: &str) -> (f64, f64, f64) {
    match room.trim().to_lowercase().as_str() {
        "small" => (0.30, 0.40, 0.5),
        "large" => (0.85, 0.25, 2.0),
        _ => (0.60, 0.35, 1.0), // medium
    }
}

fn peak(ch: &[Vec<f32>]) -> f32 {
    ch.iter().flatten().fold(0f32, |p, v| p.max(v.abs()))
}

fn scale_all(ch: &mut [Vec<f32>], g: f32) {
    ch.iter_mut().flatten().for_each(|v| *v *= g);
}

fn fade_out(ch: &mut [Vec<f32>], n: usize) {
    let len = ch.first().map_or(0, Vec::len);
    if n < 1 || len == 0 {
        return;
    }
    let n = n.min(len);
    for i in 0..n {
        let g = (n - i) as f32 / n as f32;
        for c in ch.iter_mut() {
            c[len - n + i] *= g;
        }
    }
}

// --- Freeverb network -------------------------------------------------

const COMB_TUNING: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_TUNING: [usize; 4] = [556, 441, 341, 225];

struct FreeverbNet {
    combs: Vec<Comb>,
    allpasses: Vec<Allpass>,
}

impl FreeverbNet {
    fn new(sample_rate: u32, room_size: f64, damp: f64) -> Self {
        let fb = 0.7f32 + 0.28f32 * room_size as f32; // Freeverb: room -> comb feedback
        FreeverbNet {
            combs: COMB_TUNING
                .iter()
                .map(|&t| Comb::new(scale(t, sample_rate), fb, damp as f32))
                .collect(),
            allpasses: ALLPASS_TUNING
                .iter()
                .map(|&t| Allpass::new(scale(t, sample_rate), 0.5))
                .collect(),
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let mut o = 0f32;
        for c in &mut self.combs {
            o += c.process(x); // parallel combs
        }
        for a in &mut self.allpasses {
            o = a.process(o); // series allpasses
        }
        o
    }
}

/// Tunings are specified at 44.1 kHz; scale to the actual rate.
fn scale(tuning: usize, sample_rate: u32) -> usize {
    ((tuning as f64 * sample_rate as f64 / 44100.0).round_ties_even() as usize).max(1)
}

struct Comb {
    buf: Vec<f32>,
    idx: usize,
    store: f32,
    damp1: f32,
    damp2: f32,
    fb: f32,
}

impl Comb {
    fn new(size: usize, feedback: f32, damp: f32) -> Self {
        let damp1 = damp * 0.4;
        Comb { buf: vec![0.0; size], idx: 0, store: 0.0, damp1, damp2: 1.0 - damp1, fb: feedback }
    }

    fn process(&mut self, x: f32) -> f32 {
        let o = self.buf[self.idx];
        self.store = o * self.damp2 + self.store * self.damp1; // one-pole lowpass in the feedback
        self.buf[self.idx] = x + self.store * self.fb;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        o
    }
}

struct Allpass {
    buf: Vec<f32>,
    idx: usize,
    fb: f32,
}

impl Allpass {
    fn new(size: usize, feedback: f32) -> Self {
        Allpass { buf: vec![0.0; size], idx: 0, fb: feedback }
    }

    fn process(&mut self, x: f32) -> f32 {
        let b = self.buf[self.idx];
        let o = -x + b;
        self.buf[self.idx] = x + b * self.fb;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        o
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_removes_silence_but_keeps_pad() {
        let mut ch = vec![0.0f32; 1000];
        ch[500] = 0.5;
        let out = trim(vec![ch], 1000, -50.0, 15.0);
        assert_eq!(out[0].len(), 31); // 15 pad + 1 + 15 pad
    }

    #[test]
    fn trim_leaves_pure_silence_alone() {
        let out = trim(vec![vec![0.0; 100]], 22050, -50.0, 15.0);
        assert_eq!(out[0].len(), 100);
    }

    #[test]
    fn reverb_extends_and_limits() {
        let ch = vec![vec![0.9f32; 2205]];
        let out = reverb(ch, 22050, 0.5, "small");
        assert_eq!(out[0].len(), 2205 + 11025);
        assert!(peak(&out) <= 0.99 + 1e-6);
    }
}
