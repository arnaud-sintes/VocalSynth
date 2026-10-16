# VocalSynth (`tts`)

A small Windows CLI that turns text lines into WAV files using the built-in SAPI5 voices.
Voice, speed, pitch, and volume are all adjustable, in single-line or batch mode.

## Build

```
cargo build --release
```

Output: `targetelease	ts.exe`.

The tool is written in Rust and talks to SAPI5 directly through the `windows` crate (COM `ISpVoice` / `ISpStream`), so it has no runtime dependencies beyond Windows itself.

Run the unit tests with `cargo test`.

## Usage

List the installed voices:

```
tts --list-voices
```

Single line:

```
tts --text "Shields at twenty percent." --voice Zira --out shields.wav
```

Batch (a CSV manifest of lines -> one WAV each):

```
tts --batch samples\lines.csv --outdir out
```

## Options

`--voice <substr>` matches a voice name, e.g. `Zira`, `David` (default: system default voice).
`--gender <m|f|n>` picks a voice by gender when no `--voice` matches.
`--rate <-10..10>` sets speaking speed, `0` is normal.
`--pitch <±N%>` shifts pitch via SSML, e.g. `+25%`, `-20%`.
`--volume <0..100>` sets output volume.
`--no-trim` keeps the raw silence (trimming is ON by default).
`--reverb <0..100>` sets the reverb wet mix, `0` is off.
`--reverb-room <small|medium|large>` picks the reverb room size (default `medium`).

In batch mode these flags act as defaults; each CSV row can override them per line.

## Post-processing

Every produced WAV is post-processed in this order:

1. **Trim** (on by default) removes leading/trailing silence below -50 dBFS, keeping a 15 ms guard on each side so speech attacks and decays aren't clipped.
Disable with `--no-trim`.
2. **Reverb** (off by default, enabled with `--reverb`) applies a Freeverb-style reverb, level-matched to the dry signal and extended by a decaying tail (~0.5 s / 1 s / 2 s for small / medium / large).
The output is peak-limited and given a 20 ms fade-out so the tail never clicks.

Trim runs before reverb, so SAPI's padding is removed but the reverb tail is preserved.

## Batch CSV format

The header row names the columns; only `name` and `text` are required.

```
name,text,voice,rate,pitch,volume,reverb,trim
shields_low,"Warning. Shields at twenty percent.",Zira,,,,,
hull_breach,"Hull breach on deck three.",Zira,-2,,,35,
```

Only `name` and `text` are required; the rest are optional.
`name` becomes `<outdir>\<name>.wav`.
Blank cells inherit the CLI defaults.

## Output format

Mono, 22.05 kHz, 16-bit PCM WAV (standard SAPI5 output).

## Adding better voices later

Synthesis sits behind the `TtsEngine` trait, so a higher-quality backend can be added without touching the CLI.
Candidates: Windows OneCore "natural" voices (`Windows.Media.SpeechSynthesis`), or a neural engine (Piper offline, ElevenLabs/Azure online).
