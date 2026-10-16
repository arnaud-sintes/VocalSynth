use std::fs;
use std::io::{self, Write};
use std::path::Path;

/// Reads/writes 16-bit PCM WAV files as per-channel float arrays in [-1, 1].
/// Chunk parsing skips unknown chunks (LIST, fact, ...) and only requires fmt + data.
/// Scope is what this tool produces and post-processes — 16-bit PCM — not a general WAV lib.
pub struct Audio {
    pub sample_rate: u32,
    /// `[channel][sample]`
    pub channels: Vec<Vec<f32>>,
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

pub fn read(path: &Path) -> io::Result<Audio> {
    let b = fs::read(path)?;
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(invalid(format!("not a RIFF/WAVE file: {}", path.display())));
    }

    let (mut channels, mut sample_rate, mut bits, mut format) = (0usize, 0u32, 0u16, 0u16);
    let mut data: Option<(usize, usize)> = None; // (offset, length)

    let mut pos = 12usize;
    while pos + 8 <= b.len() {
        let id = &b[pos..pos + 4];
        let size = u32_at(&b, pos + 4).unwrap_or(0) as usize;
        let body = pos + 8;

        if id == b"fmt " {
            let trunc = || invalid("truncated fmt chunk.".into());
            format = u16_at(&b, body).ok_or_else(trunc)?;
            channels = u16_at(&b, body + 2).ok_or_else(trunc)? as usize;
            sample_rate = u32_at(&b, body + 4).ok_or_else(trunc)?;
            bits = u16_at(&b, body + 14).ok_or_else(trunc)?;
        } else if id == b"data" {
            data = Some((body, size.min(b.len().saturating_sub(body))));
        }

        pos = body.saturating_add(size).saturating_add(size & 1); // chunks are word-aligned
    }

    if format != 1 && format != 0xFFFE {
        return Err(invalid(format!("only PCM WAV is supported (format={format}).")));
    }
    if bits != 16 {
        return Err(invalid(format!("only 16-bit PCM is supported (bits={bits}).")));
    }
    let (data_off, data_len) = match data {
        Some(d) if channels >= 1 => d,
        _ => return Err(invalid("missing fmt/data chunk.".into())),
    };

    let frames = data_len / (2 * channels);
    let mut result = vec![Vec::with_capacity(frames); channels];
    let samples = b[data_off..data_off + frames * channels * 2]
        .chunks_exact(2)
        .map(|s| i16::from_le_bytes([s[0], s[1]]));
    for (i, s) in samples.enumerate() {
        result[i % channels].push(s as f32 / 32768.0);
    }

    Ok(Audio { sample_rate, channels: result })
}

pub fn write(path: &Path, sample_rate: u32, channels: &[Vec<f32>]) -> io::Result<()> {
    let nch = channels.len();
    let frames = channels.first().map_or(0, Vec::len);
    let block_align = (nch * 2) as u32;
    let data_len = frames as u32 * block_align;

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&(nch as u16).to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * block_align).to_le_bytes()); // byte rate
    out.extend_from_slice(&(block_align as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes()); // bits

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());

    for f in 0..frames {
        for ch in channels {
            let s = (ch[f] * 32767.0).round_ties_even().clamp(-32768.0, 32767.0) as i16;
            out.extend_from_slice(&s.to_le_bytes());
        }
    }

    fs::File::create(path)?.write_all(&out)
}
