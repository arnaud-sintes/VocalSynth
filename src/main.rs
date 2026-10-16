mod args;
mod csv;
mod engine;
mod fx;
mod request;
mod sapi5;
mod wav;

use std::path::{self, Path, PathBuf};
use std::process::ExitCode;

use args::{parse_int, Args};
use engine::{Result, TtsEngine};
use request::SpeechRequest;
use sapi5::Sapi5Engine;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            if std::env::var("TTS_DEBUG").as_deref() == Ok("1") {
                eprintln!("{e:?}");
            }
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<u8> {
    let opts = Args::parse(args)?;

    if opts.show_help {
        print_help();
        return Ok(0);
    }

    let engine = Sapi5Engine::new()?;

    if opts.list_voices {
        print_voices(&engine)?;
        return Ok(0);
    }

    if opts.batch_path.is_some() {
        return run_batch(&engine, &opts);
    }

    run_single(&engine, &opts)
}

fn run_single(engine: &dyn TtsEngine, opts: &Args) -> Result<u8> {
    let text = opts.text.as_deref().filter(|t| !t.trim().is_empty()).ok_or(
        "--text is required (or use --batch / --list-voices). See --help.",
    )?;
    let out = opts.out.as_deref().filter(|o| !o.trim().is_empty()).ok_or(
        "--out <file.wav> is required in single mode. See --help.",
    )?;

    let req = SpeechRequest {
        text: text.to_string(),
        output_path: path::absolute(out)?,
        voice_pattern: opts.voice.clone(),
        gender: opts.gender,
        rate: opts.rate,
        pitch: opts.pitch.clone(),
        volume: opts.volume,
        trim: opts.trim,
        reverb: opts.reverb,
        reverb_room: opts.reverb_room.clone(),
    };

    engine.synthesize(&req)?;
    post_process(&req)?;
    println!("wrote {}", req.output_path.display());
    Ok(0)
}

fn run_batch(engine: &dyn TtsEngine, opts: &Args) -> Result<u8> {
    let batch_path = opts.batch_path.as_deref().unwrap_or_default();
    let outdir = path::absolute(opts.out_dir.as_deref().unwrap_or("."))?;
    let rows = csv::parse(&read_text_file(Path::new(batch_path))?);
    if rows.is_empty() {
        return Err(format!("batch file is empty: {batch_path}").into());
    }

    let header: Vec<String> = rows[0].iter().map(|h| h.trim().to_lowercase()).collect();
    let col = |name: &str| header.iter().position(|h| h == name);
    let (Some(name_col), Some(text_col)) = (col("name"), col("text")) else {
        return Err("batch CSV must have at least 'name' and 'text' columns.".into());
    };
    let (voice_col, rate_col, pitch_col, vol_col) =
        (col("voice"), col("rate"), col("pitch"), col("volume"));
    let (reverb_col, trim_col) = (col("reverb"), col("trim"));

    let cell = |r: &[String], col: Option<usize>| -> Option<String> {
        col.and_then(|c| r.get(c))
            .filter(|v| !v.trim().is_empty())
            .map(|v| v.trim().to_string())
    };
    let int_cell = |r: &[String], col: Option<usize>, what: &str, default: i32| -> Result<i32> {
        Ok(match cell(r, col) {
            Some(v) => parse_int(what, &v)?,
            None => default,
        })
    };

    let (mut ok, mut failed) = (0, 0);
    for (i, r) in rows.iter().enumerate().skip(1) {
        let line = i + 1;
        let (name, text) = (cell(r, Some(name_col)), cell(r, Some(text_col)));
        let (name, text) = match (name, text) {
            (None, None) => continue, // tolerate blank lines
            (Some(n), Some(t)) => (n, t),
            _ => {
                eprintln!("  line {line}: skipped — missing name or text");
                failed += 1;
                continue;
            }
        };

        let file_name = if name.to_lowercase().ends_with(".wav") {
            name.clone()
        } else {
            format!("{name}.wav")
        };
        let req = SpeechRequest {
            text,
            output_path: outdir.join(&file_name),
            voice_pattern: cell(r, voice_col).or_else(|| opts.voice.clone()),
            gender: opts.gender,
            rate: int_cell(r, rate_col, "rate", opts.rate)?,
            pitch: cell(r, pitch_col).unwrap_or_else(|| opts.pitch.clone()),
            volume: int_cell(r, vol_col, "volume", opts.volume)?,
            reverb: int_cell(r, reverb_col, "reverb", opts.reverb)?,
            trim: match cell(r, trim_col) {
                Some(v) => parse_bool(&v)?,
                None => opts.trim,
            },
            reverb_room: opts.reverb_room.clone(),
        };

        match engine.synthesize(&req).and_then(|()| post_process(&req)) {
            Ok(()) => {
                println!("  {file_name}");
                ok += 1;
            }
            Err(e) => {
                eprintln!("  line {line} ({name}): {e}");
                failed += 1;
            }
        }
    }

    println!("done: {ok} written, {failed} failed  ->  {}", outdir.display());
    Ok(if failed == 0 { 0 } else { 2 })
}

/// Apply the trim + reverb DSP chain to a just-written WAV, in place.
fn post_process(req: &SpeechRequest) -> Result<()> {
    if !req.trim && req.reverb <= 0 {
        return Ok(()); // nothing to do
    }

    let audio = wav::read(&req.output_path)?;
    let rate = audio.sample_rate;
    let mut ch = audio.channels;
    if req.trim {
        ch = fx::trim(ch, rate, -50.0, 15.0);
    }
    if req.reverb > 0 {
        ch = fx::reverb(ch, rate, f64::from(req.reverb) / 100.0, &req.reverb_room);
    }
    wav::write(&req.output_path, rate, &ch)?;
    Ok(())
}

/// Reads a text file, honoring a UTF-8 or UTF-16 BOM; anything else is decoded as UTF-8
/// with invalid bytes replaced, so a stray legacy-encoded character never aborts a batch.
fn read_text_file(path: &Path) -> Result<String> {
    let b = std::fs::read(path)
        .map_err(|e| format!("cannot read {}: {e}", PathBuf::from(path).display()))?;
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    };
    Ok(match b.as_slice() {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => utf16(rest, false),
        _ => String::from_utf8_lossy(&b).into_owned(),
    })
}

fn parse_bool(s: &str) -> Result<bool> {
    match s.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" => Ok(false),
        _ => Err(format!("invalid boolean '{s}' (use true/false)").into()),
    }
}

fn print_voices(engine: &dyn TtsEngine) -> Result<()> {
    println!("Engine: {}", engine.name());
    for v in engine.list_voices()? {
        println!("  {:<40} {:<8} {}", v.name, v.gender, v.culture);
    }
    Ok(())
}

fn print_help() {
    println!(
        "\
tts — text-to-speech WAV generator (SAPI5 backend)

SINGLE:
  tts --text \"Shields at twenty percent.\" --out shields.wav [options]

BATCH:
  tts --batch lines.csv --outdir ./out [default options]
    CSV columns: name,text[,voice,rate,pitch,volume,reverb,trim]
    Per-row cells override the CLI defaults; blank cells inherit them.
    'name' becomes <outdir>/<name>.wav

OPTIONS (defaults for single mode / fallbacks for batch):
  --voice       <substr>   voice name match, e.g. Zira, David  (default: system default)
  --gender      <m|f|n>    used when no --voice matches
  --rate        <-10..10>  speaking speed, 0 = normal           (default: 0)
  --pitch       <±N%>      pitch shift via SSML, e.g. +25%      (default: +0%)
  --volume      <0..100>   output volume                        (default: 100)
  --no-trim                keep silence (trimming is ON by default)
  --reverb      <0..100>   reverb wet mix, 0 = off              (default: 0)
  --reverb-room <name>     small | medium | large               (default: medium)

OTHER:
  --list-voices        list installed voices and exit
  --help               this text"
    );
}
