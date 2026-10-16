//! SAPI5 backend built on the native COM API (`ISpVoice`). Writes WAV natively via an
//! `ISpStream` bound to a file. Rate and volume map to native properties; pitch is
//! applied through SSML prosody, which SAPI5 has no direct API for.

use std::fs;
use std::mem::ManuallyDrop;

use windows::core::{w, Interface, GUID, PCWSTR};
use windows::Win32::Globalization::LCIDToLocaleName;
use windows::Win32::Media::Audio::WAVEFORMATEX;
use windows::Win32::Media::Speech::{
    IEnumSpObjectTokens, ISpDataKey, ISpObjectToken, ISpObjectTokenCategory, ISpStream, ISpVoice,
    SPCAT_VOICES, SPFM_CREATE_ALWAYS, SPF_IS_XML,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};

use crate::engine::{Result, TtsEngine};
use crate::request::{SpeechRequest, VoiceDescriptor, VoiceGender};

// Coclass IDs and the WAVEFORMATEX format ID live in the SAPI type library, not in the
// Win32 metadata, so they are spelled out here.
const CLSID_SP_VOICE: GUID = GUID::from_u128(0x96749377_3391_11d2_9ee3_00c04f797396);
const CLSID_SP_STREAM: GUID = GUID::from_u128(0x715d9c59_4442_11d2_9605_00c04f8ee628);
const CLSID_SP_OBJECT_TOKEN_CATEGORY: GUID = GUID::from_u128(0xa910187f_0c7a_45ac_92cc_59edafb77b53);
const SPDFID_WAVE_FORMAT_EX: GUID = GUID::from_u128(0xc31adbae_527f_4ff5_a230_f62bb61ff70c);

/// Output format of the produced WAV: mono, 22.05 kHz, 16-bit PCM (standard SAPI5 output).
const WAVE_FORMAT: WAVEFORMATEX = WAVEFORMATEX {
    wFormatTag: 1, // WAVE_FORMAT_PCM
    nChannels: 1,
    nSamplesPerSec: 22050,
    nAvgBytesPerSec: 22050 * 2,
    nBlockAlign: 2,
    wBitsPerSample: 16,
    cbSize: 0,
};

pub struct Sapi5Engine {
    voice: ManuallyDrop<ISpVoice>,
}

impl Sapi5Engine {
    pub fn new() -> Result<Self> {
        // SAFETY: plain COM initialization; balanced by CoUninitialize in Drop. An
        // already-initialized apartment (S_FALSE) is fine and also needs the matching call.
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            match CoCreateInstance(&CLSID_SP_VOICE, None, CLSCTX_ALL) {
                Ok(voice) => Ok(Sapi5Engine { voice: ManuallyDrop::new(voice) }),
                Err(e) => {
                    CoUninitialize();
                    Err(e.into())
                }
            }
        }
    }

    /// Enumerate installed voice tokens, optionally filtered by SAPI attribute requirements
    /// such as `Gender=Male`.
    fn tokens(&self, required: Option<PCWSTR>) -> Result<Vec<ISpObjectToken>> {
        unsafe {
            let cat: ISpObjectTokenCategory =
                CoCreateInstance(&CLSID_SP_OBJECT_TOKEN_CATEGORY, None, CLSCTX_ALL)?;
            cat.SetId(SPCAT_VOICES, false)?;
            let list: IEnumSpObjectTokens =
                cat.EnumTokens(required.unwrap_or(PCWSTR::null()), PCWSTR::null())?;
            let mut count = 0u32;
            list.GetCount(&mut count)?;
            (0..count).map(|i| list.Item(i).map_err(Into::into)).collect()
        }
    }

    /// Pick a voice by name substring first, then by gender, else leave the default.
    /// Fails only when a name pattern or gender was given but matched nothing — a silent
    /// fallback there would quietly produce the wrong voice.
    fn select_voice(&self, request: &SpeechRequest) -> Result<()> {
        if let Some(pattern) = request.voice_pattern.as_deref().filter(|p| !p.trim().is_empty()) {
            let tokens = self.tokens(None)?;
            let needle = pattern.to_lowercase();
            let found = tokens
                .iter()
                .find(|t| voice_name(t).to_lowercase().contains(&needle));
            return match found {
                Some(t) => unsafe { Ok(self.voice.SetVoice(t)?) },
                None => {
                    let names: Vec<String> = tokens.iter().map(voice_name).collect();
                    Err(format!(
                        "No installed voice matches \"{pattern}\". Available: {}",
                        names.join(", ")
                    )
                    .into())
                }
            };
        }

        if let Some(g) = request.gender {
            let req = match g {
                VoiceGender::Male => w!("Gender=Male"),
                VoiceGender::Female => w!("Gender=Female"),
                VoiceGender::Neutral => w!("Gender=Neutral"),
            };
            let tokens = self.tokens(Some(req))?;
            let Some(t) = tokens.first() else {
                return Err(format!("No installed voice has gender {g}.").into());
            };
            unsafe { self.voice.SetVoice(t)? };
        }
        Ok(())
    }

    /// Culture of the currently selected voice, used for the SSML `xml:lang`.
    fn current_culture(&self) -> String {
        unsafe { self.voice.GetVoice() }
            .ok()
            .and_then(|t| attribute(&t, "Language"))
            .map(|l| culture_name(&l))
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| "en-US".to_string())
    }
}

impl TtsEngine for Sapi5Engine {
    fn name(&self) -> &str {
        "SAPI5 (native)"
    }

    fn list_voices(&self) -> Result<Vec<VoiceDescriptor>> {
        Ok(self
            .tokens(None)?
            .iter()
            .map(|t| VoiceDescriptor {
                name: voice_name(t),
                gender: match attribute(t, "Gender").map(|g| g.to_lowercase()).as_deref() {
                    Some("male") => VoiceGender::Male,
                    Some("female") => VoiceGender::Female,
                    _ => VoiceGender::Neutral,
                },
                culture: attribute(t, "Language")
                    .map(|l| culture_name(&l))
                    .filter(|c| !c.is_empty())
                    .unwrap_or_else(|| "unknown".to_string()),
            })
            .collect())
    }

    fn synthesize(&self, request: &SpeechRequest) -> Result<()> {
        self.select_voice(request)?;

        if let Some(dir) = request.output_path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }

        let ssml = build_ssml(request, &self.current_culture());
        let path: Vec<u16> = request
            .output_path
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let ssml: Vec<u16> = ssml.encode_utf16().chain(Some(0)).collect();

        unsafe {
            self.voice.SetRate(request.rate.clamp(-10, 10))?;
            self.voice.SetVolume(request.volume.clamp(0, 100) as u16)?;

            let stream: ISpStream = CoCreateInstance(&CLSID_SP_STREAM, None, CLSCTX_ALL)?;
            stream.BindToFile(
                PCWSTR(path.as_ptr()),
                SPFM_CREATE_ALWAYS,
                Some(&SPDFID_WAVE_FORMAT_EX),
                Some(&WAVE_FORMAT),
                0,
            )?;
            self.voice.SetOutput(&stream, true)?;

            let spoken = self
                .voice
                .Speak(PCWSTR(ssml.as_ptr()), SPF_IS_XML.0 as u32, None)
                .and_then(|_| self.voice.WaitUntilDone(u32::MAX));

            // Release the file handle so the WAV header is finalized and readable immediately.
            let _ = self.voice.SetOutput(None::<&windows::core::IUnknown>, true);
            let closed = stream.Close();
            spoken?;
            closed?;
        }
        Ok(())
    }
}

impl Drop for Sapi5Engine {
    fn drop(&mut self) {
        // Release the COM object before tearing down the apartment it lives in.
        unsafe {
            ManuallyDrop::drop(&mut self.voice);
            CoUninitialize();
        }
    }
}

fn build_ssml(request: &SpeechRequest, culture: &str) -> String {
    // Escape the text so quotes / ampersands / angle brackets in a line don't break the SSML.
    format!(
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='{}'>\
         <prosody pitch='{}'>{}</prosody></speak>",
        xml_escape(culture),
        xml_escape(&request.pitch),
        xml_escape(&request.text),
    )
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Reads a string value from a data key; `None` when absent or empty.
fn read_string(key: &ISpDataKey, name: PCWSTR) -> Option<String> {
    unsafe {
        let p = key.GetStringValue(name).ok()?;
        if p.is_null() {
            return None;
        }
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.filter(|s| !s.is_empty())
    }
}

/// A voice's `Attributes\<name>` value (Name, Gender, Language, ...).
fn attribute(token: &ISpObjectToken, name: &str) -> Option<String> {
    let attrs: ISpDataKey = unsafe { token.OpenKey(w!("Attributes")) }.ok()?;
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    read_string(&attrs, PCWSTR(wide.as_ptr()))
}

fn voice_name(token: &ISpObjectToken) -> String {
    attribute(token, "Name")
        .or_else(|| read_string(token.cast::<ISpDataKey>().as_ref().ok()?, PCWSTR::null()))
        .unwrap_or_else(|| "unknown".to_string())
}

/// Converts a SAPI `Language` attribute (hex LCID, optionally `;`-separated, e.g. "409;9")
/// to a culture name such as "en-US".
fn culture_name(language: &str) -> String {
    let first = language.split(';').next().unwrap_or("").trim();
    let Ok(lcid) = u32::from_str_radix(first, 16) else {
        return String::new();
    };
    let mut buf = [0u16; 85];
    let n = unsafe { LCIDToLocaleName(lcid, Some(&mut buf), 0) };
    if n <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..(n - 1) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_xml_specials() {
        assert_eq!(xml_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }

    #[test]
    fn culture_from_lcid() {
        assert_eq!(culture_name("409;9"), "en-US");
        assert_eq!(culture_name("40C"), "fr-FR");
        assert_eq!(culture_name("zz"), "");
    }
}
