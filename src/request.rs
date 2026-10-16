use std::fmt;
use std::path::PathBuf;

/// A single synthesis job: the text to speak, how to speak it, and where to write the WAV.
/// Engine-agnostic — every backend consumes this same shape.
#[derive(Debug, Clone)]
pub struct SpeechRequest {
    /// Text to synthesize.
    pub text: String,
    /// Absolute path of the WAV file to produce.
    pub output_path: PathBuf,
    /// Case-insensitive substring matched against installed voice names (e.g. "Zira").
    /// `None` means "no name preference" — selection falls back to `gender`.
    pub voice_pattern: Option<String>,
    /// Preferred gender when no `voice_pattern` is given. `None` = don't care.
    pub gender: Option<VoiceGender>,
    /// Speaking rate, -10 (slowest) ..= +10 (fastest). 0 = normal.
    pub rate: i32,
    /// Relative pitch shift applied via SSML prosody, e.g. "+25%", "-20%", "+0%".
    pub pitch: String,
    /// Output volume, 0..=100.
    pub volume: i32,
    /// Trim leading/trailing silence from the produced WAV. On by default.
    pub trim: bool,
    /// Reverb wet mix, 0..=100. 0 = no reverb.
    pub reverb: i32,
    /// Reverb room preset: "small" | "medium" | "large". Only used when `reverb` > 0.
    pub reverb_room: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceGender {
    Male,
    Female,
    Neutral,
}

impl fmt::Display for VoiceGender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(match self {
            VoiceGender::Male => "Male",
            VoiceGender::Female => "Female",
            VoiceGender::Neutral => "Neutral",
        })
    }
}

/// Describes an installed voice.
#[derive(Debug, Clone)]
pub struct VoiceDescriptor {
    pub name: String,
    pub gender: VoiceGender,
    pub culture: String,
}
