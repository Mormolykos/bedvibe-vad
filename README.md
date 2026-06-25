# bedvibe-vad — Real-Time Voice-Activity Detection (Rust)

A small, self-contained Rust binary that captures microphone audio, detects
speech activity, and streams transcriptions into a real-time application. Built
as the audio front-end of the BedVibe AI companion — it runs as its own process
so the heavy audio work stays out of the game engine.

## What it does

- Captures live microphone audio with **cpal**.
- Buffers and segments speech into temporary WAV files (**hound**).
- Uploads each segment to a **Whisper** ASR endpoint via **reqwest** multipart.
- Bridges the returned transcription back to the host app (Unity) over a
  **local TCP** socket.
- Exposes a **start / mute control port** so the host can gate listening — e.g.
  to stop the system transcribing its own generated voice.
- Cleans up temporary audio files after each turn.

## Stack

Rust · cpal · hound · reqwest (multipart) · Whisper ASR · TCP (localhost)

## Configuration

All settings are optional — copy `.env.example` to `.env` and override any of these (or set them as real environment variables):

| Variable | Default | Meaning |
|---|---|---|
| `VAD_WHISPER_URL` | `http://127.0.0.1:8000/inference` | ASR endpoint that receives each WAV segment |
| `VAD_UNITY_PORT` | `5005` | TCP port this tool sends transcriptions to |
| `VAD_MUTE_PORT` | `5006` | TCP port this tool listens on for `start` / `mute` |
| `VAD_THRESHOLD_DB` | `-30.0` | Voice-activity threshold in dB |
| `VAD_SILENCE_MS` | `1200` | Silence (ms) before a segment is finalized |

## Build

```sh
cargo build --release
```

The binary is produced in `target/release/`. (`target/` is gitignored — it
regenerates on build.)
