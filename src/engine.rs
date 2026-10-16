use crate::request::{SpeechRequest, VoiceDescriptor};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A text-to-speech backend. SAPI5 is the first implementation; OneCore / neural
/// engines can be added later behind this same trait without touching the CLI.
pub trait TtsEngine {
    /// Human-readable engine name, shown in `--list-voices` output.
    fn name(&self) -> &str;

    /// Voices this engine exposes.
    fn list_voices(&self) -> Result<Vec<VoiceDescriptor>>;

    /// Render one request to its WAV file. Creates the output directory if needed.
    fn synthesize(&self, request: &SpeechRequest) -> Result<()>;
}
