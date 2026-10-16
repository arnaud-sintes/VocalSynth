use crate::request::VoiceGender;

/// Parsed command-line options. Thin, hand-rolled — no external arg library needed.
#[derive(Debug)]
pub struct Args {
    pub show_help: bool,
    pub list_voices: bool,

    pub text: Option<String>,
    pub out: Option<String>,
    pub batch_path: Option<String>,
    pub out_dir: Option<String>,

    pub voice: Option<String>,
    pub gender: Option<VoiceGender>,
    pub rate: i32,
    pub pitch: String,
    pub volume: i32,
    pub trim: bool,
    pub reverb: i32,
    pub reverb_room: String,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            show_help: false,
            list_voices: false,
            text: None,
            out: None,
            batch_path: None,
            out_dir: None,
            voice: None,
            gender: None,
            rate: 0,
            pitch: "+0%".to_string(),
            volume: 100,
            trim: true,
            reverb: 0,
            reverb_room: "medium".to_string(),
        }
    }
}

impl Args {
    pub fn parse(args: &[String]) -> Result<Args, String> {
        let mut o = Args::default();
        if args.is_empty() {
            o.show_help = true;
            return Ok(o);
        }

        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut next = || {
                it.next()
                    .cloned()
                    .ok_or_else(|| format!("{a} needs a value"))
            };

            match a.as_str() {
                "--help" | "-h" | "/?" => o.show_help = true,
                "--list-voices" => o.list_voices = true,
                "--text" => o.text = Some(next()?),
                "--out" => o.out = Some(next()?),
                "--batch" => o.batch_path = Some(next()?),
                "--outdir" => o.out_dir = Some(next()?),
                "--voice" => o.voice = Some(next()?),
                "--gender" => o.gender = Some(parse_gender(&next()?)?),
                "--rate" => o.rate = parse_int(a, &next()?)?,
                "--pitch" => o.pitch = normalize_pitch(&next()?)?,
                "--volume" => o.volume = parse_int(a, &next()?)?,
                "--no-trim" => o.trim = false,
                "--reverb" => o.reverb = parse_int(a, &next()?)?,
                "--reverb-room" => o.reverb_room = validate_room(&next()?)?,
                _ => return Err(format!("unknown argument: {a} (see --help)")),
            }
        }

        Ok(o)
    }
}

/// Parses an integer (surrounding whitespace allowed); `what` names the flag or CSV column in the error.
pub fn parse_int(what: &str, s: &str) -> Result<i32, String> {
    s.trim()
        .parse::<i32>()
        .map_err(|_| format!("invalid value '{s}' for {what} (expected an integer)"))
}

fn validate_room(s: &str) -> Result<String, String> {
    let r = s.trim().to_lowercase();
    match r.as_str() {
        "small" | "medium" | "large" => Ok(r),
        _ => Err(format!("invalid --reverb-room '{s}' (use small|medium|large)")),
    }
}

fn parse_gender(s: &str) -> Result<VoiceGender, String> {
    match s.trim().to_lowercase().as_str() {
        "m" | "male" => Ok(VoiceGender::Male),
        "f" | "female" => Ok(VoiceGender::Female),
        "n" | "neutral" => Ok(VoiceGender::Neutral),
        _ => Err(format!("invalid --gender '{s}' (use m|f|n)")),
    }
}

/// Accepts "+25%", "-20%", "25", "-20" — normalizes to the SSML "+N%"/"-N%" form.
fn normalize_pitch(s: &str) -> Result<String, String> {
    let s = s.trim();
    let s = s.strip_suffix('%').unwrap_or(s);
    let n: i32 = s
        .parse()
        .map_err(|_| format!("invalid --pitch '{s}' (use e.g. +25%, -20%, 0)"))?;
    Ok(format!("{}{}%", if n >= 0 { "+" } else { "" }, n))
}
