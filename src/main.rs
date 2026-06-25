use std::{
    error::Error,
    fs::{File, remove_file},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat,
    StreamConfig,
};

use hound::{SampleFormat as HoundSample, WavSpec, WavWriter};
use uuid::Uuid;

// reqwest blocking + multipart
use reqwest::blocking::{
    Client,
    multipart::{Form, Part},
};

const THRESHOLD_DB:    f32 = -30.0;     // voice threshold
const SILENCE_MS:      u64 = 1_200;    // ms of silence to finalize
const MUTE_PORT:       u16 = 5006;     // Unity → VAT commands
const UNITY_PORT:      u16 = 5005;     // VAT → Unity transcriptions

fn amplitude_to_db(sample: f32) -> f32 {
    let amp = sample.abs().max(1e-6);
    20.0 * amp.log10()
}

fn write_wav(filepath: &str, samples: &[f32], sample_rate: u32) {
    if let Some(dir) = Path::new(filepath).parent() {
        let _ = std::fs::create_dir_all(dir);
		println!("🛠 write_wav() called: path={}, samples={}, rate={}", filepath, samples.len(), sample_rate);
    }

    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: HoundSample::Int,
    };

    let mut writer = WavWriter::create(filepath, spec)
        .expect("Failed to create WAV writer");
		println!("✅ WavWriter created");

    for &s in samples {
        let c = s.clamp(-1.0, 1.0);
        let i = (c * i16::MAX as f32) as i16;
        writer.write_sample(i).unwrap();
    }
    println!("📦 Finishing WAV write...");
    writer.finalize().expect("❌ Failed to finalize WAV");
}

fn send_transcription_to_unity(text: &str) {
    if let Ok(mut sock) = TcpStream::connect(("127.0.0.1", UNITY_PORT)) {
        let body = text;
        let req = format!(
            "POST /transcription HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Length: {}\r\n\r\n\
             {}",
            body.len(),
            body
        );
        let _ = sock.write_all(req.as_bytes());
    } else {
        eprintln!("⚠ Could not connect to Unity on port {}", UNITY_PORT);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // args: <mic_name_substring> <temp_folder>
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("Usage: vat <mic_name> <temp_folder>");
        return Ok(());
    }
    let mic_name    = &args[1];
    let temp_folder = &args[2];

    println!(">>> VAT main() entry point reached");

    // pick the named input device
    let host   = cpal::default_host();
    let device = host.input_devices()?
        .find(|d| d.name().ok().map_or(false, |n| n.contains(mic_name)))
        .expect("Microphone not found");

    let cfg          = device.default_input_config()?;
    let sample_rate  = cfg.sample_rate().0;
    let channels_in  = cfg.channels() as usize;
    let stream_cfg   : StreamConfig = cfg.clone().into();

    println!(
        "🎧 Using '{}' @ {} Hz, {} channels",
        device.name()?, sample_rate, channels_in
    );

    // shared state
    let recording   = Arc::new(Mutex::new(false));
    let last_spoke  = Arc::new(Mutex::new(Instant::now()));
    let sample_buf  = Arc::new(Mutex::new(Vec::<f32>::new()));

    // a little TCP server to receive "start"/"mute" from Unity
    {
        let rec_flag = recording.clone();
        let last     

= last_spoke.clone();
        thread::spawn(move || {
            let listener = TcpListener::bind(("127.0.0.1", MUTE_PORT))
                .expect("Failed to bind mute port");
            for conn in listener.incoming() {
                if let Ok(mut sock) = conn {
                    let mut buf = [0u8; 128];
                    if let Ok(n) = sock.read(&mut buf) {
                        let msg = String::from_utf8_lossy(&buf[..n])
                            .trim()
                            .to_lowercase();
                        match msg.as_str() {
                            "start" => {
                                *rec_flag.lock().unwrap() = true;
                                *last     

.lock().unwrap()        = Instant::now();
                            }
                            "mute" => {
                                *rec_flag.lock().unwrap() = false;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
    }

    let err_fn = |err| eprintln!("Stream error: {:?}", err);

    // build & start input stream
    let buf_cl = sample_buf.clone();
    let rec_cl = recording.clone();
    let lv_cl  = last_spoke.clone();

    let stream = match cfg.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &stream_cfg,
            move |data: &[f32], _| {
                let mut buf  = buf_cl.lock().unwrap();
                let mut rec  = rec_cl.lock().unwrap();
                let mut last = lv_cl.lock().unwrap();
                for frame in data.chunks(channels_in) {
                    let s  = frame[0];
                    let db = amplitude_to_db(s);
                    if db > THRESHOLD_DB {
                        if !*rec {
                            *rec = true;
                            *last = Instant::now();
                        } else {
                            *last = Instant::now();
                        }
                    }
                    if *rec {
                        buf.push(s);
                    }
                }
            },
            err_fn,
            None,
        )?,
        SampleFormat::I16 => device.build_input_stream(
            &stream_cfg,
            move |data: &[i16], _| {
                let mut buf  = buf_cl.lock().unwrap();
                let mut rec  = rec_cl.lock().unwrap();
                let mut last = lv_cl.lock().unwrap();
                for chunk in data.chunks(channels_in) {
                    let s  = chunk[0] as f32 / i16::MAX as f32;
                    let db = amplitude_to_db(s);
                    if db > THRESHOLD_DB {
                        if !*rec {
                            *rec = true;
                            *last = Instant::now();
                        } else {
                            *last = Instant::now();
                        }
                    }
                    if *rec {
                        buf.push(s);
                    }
                }
            },
            err_fn,
            None,
        )?,
        other => panic!("Unsupported format: {:?}", other),
    };

    stream.play()?;
    println!("🟢 Listening… speak, then pause to save.");

    // main “silence detector → flush → send to Whisper → relay” loop
    loop {
        thread::sleep(Duration::from_millis(100));
        if !*recording.lock().unwrap() {
            continue;
        }
        let last_time = *last_spoke.lock().unwrap();
        if last_time.elapsed().as_millis() as u64 > SILENCE_MS {
            // finalize
            let mut buf = sample_buf.lock().unwrap();
            if buf.is_empty() {
                *recording.lock().unwrap() = false;
                continue;
            }
            let id = Uuid::new_v4();
            let filename = format!("{}/{}.wav", temp_folder, id);
                println!(
                "🔇 Silence of {:.1}s → saving {}",
                last_time.elapsed().as_secs_f32(),
                filename
        );

write_wav(&filename, &buf, sample_rate);
println!("✅ Saved WAV: {}", filename);
buf.clear();
*recording.lock().unwrap() = false;


            // send to Whisper
let client = Client::new();

// 1) open the WAV file
let file = File::open(&filename).map_err(|e| {
    eprintln!("⚠️ could not open {}: {}", &filename, e);
    e
})?;

// 2) extract just the filename for the multipart
let file_name_str = Path::new(&filename)
    .file_name()
    .and_then(|n| n.to_str())
    .unwrap_or("audio.wav")
    .to_string();  
	
// 3) build a Part with the right file name & Content-Type
let audio_part = Part::reader(file)
    .file_name(file_name_str)
    .mime_str("audio/wav")
    .expect("invalid MIME");

// 4) assemble the multipart form
let form = Form::new()
    .part("file", audio_part)            // Whisper expects `file`
    .text("model", "whisper-1")          // or your chosen model
    .text("language", "en")              // optional
    .text("temperature", "0.0");         // optional

// 5) POST to your local Whisper server
let resp = client
    .post("http://127.0.0.1:8000/inference")
    .multipart(form)
    .send()
    .map_err(|e| {
        eprintln!("⚠️ Whisper request failed: {}", e);
        e
    })?
    .error_for_status()
    .map_err(|e| {
        eprintln!("⚠️ Whisper returned non-OK: {}", e);
        e
    })?;

// 6) pull out the transcription text
let text = resp.text().map_err(|e| {
    eprintln!("⚠️ Failed to read Whisper response: {}", e);
    e
})?;

// 7) forward it on
println!(
    "{{\"type\":\"transcription\",\"text\":{:?},\"file\":{:?}}}",
    text, filename
);
send_transcription_to_unity(&text);

            // 8) delete the WAV so your temp folder never fills up
            if let Err(e) = remove_file(&filename) {
                eprintln!("⚠️ could not delete {}: {}", &filename, e);
            }   // <-- closes the remove_file if‐block

        }   // <-- ADD THIS to close the `if last_time.elapsed()… {` block

    }   // <-- closes the `loop { … }`

    // (no more code—loop never exits, main returns !)
}   // <-- closes `fn main()`

