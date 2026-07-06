//! CLI demo for yeli:
//!   yeli list
//!   yeli info  <plugin-uri>
//!   yeli run   <plugin-uri> [out.wav] [seconds] [midi-note]
//!
//! `run` renders offline: instruments get a MIDI note, effects get a 440 Hz
//! sine on their audio inputs. Output is written as a 32-bit float WAV.

use yeli::{PortDirection, PortKind, World};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK: usize = 512;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(),
        Some("info") if args.len() >= 2 => cmd_info(&args[1]),
        Some("run") if args.len() >= 2 => cmd_run(&args[1..]),
        _ => {
            eprintln!("usage:");
            eprintln!("  yeli list");
            eprintln!("  yeli info <plugin-uri>");
            eprintln!("  yeli run  <plugin-uri> [out.wav] [seconds] [midi-note]");
            std::process::exit(2);
        }
    }
}

fn cmd_list() {
    let world = World::discover();
    for p in &world.plugins {
        println!("{}\t{}", p.uri, p.name);
    }
    eprintln!("({} plugins)", world.plugins.len());
}

fn cmd_info(uri: &str) {
    let world = World::discover();
    let Some(p) = world.plugin_by_uri(uri) else {
        eprintln!("plugin not found: {uri}");
        std::process::exit(1);
    };
    println!("name    : {}", p.name);
    println!("uri     : {}", p.uri);
    println!("bundle  : {}", p.bundle_path.display());
    println!("binary  : {}", p.binary_path.display());
    if !p.required_features.is_empty() {
        println!("requires: {}", p.required_features.join(", "));
    }
    println!("ports   :");
    for port in &p.ports {
        println!(
            "  [{:>3}] {:<6} {:<12} {:<24} {}  (default {:?}, range {:?}..{:?})",
            port.index,
            match port.direction {
                PortDirection::Input => "in",
                PortDirection::Output => "out",
            },
            format!("{:?}", port.kind),
            port.symbol,
            port.name,
            port.default,
            port.minimum,
            port.maximum,
        );
    }
}

fn cmd_run(args: &[String]) {
    let uri = &args[0];
    let out_path = args.get(1).cloned().unwrap_or_else(|| "out.wav".to_string());
    let seconds: f64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2.0);
    let note: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(60);

    let world = World::discover();
    let Some(plugin) = world.plugin_by_uri(uri) else {
        eprintln!("plugin not found: {uri}");
        std::process::exit(1);
    };
    println!("loading: {} ({})", plugin.name, plugin.uri);

    let mut instance = match world.instantiate(plugin, SAMPLE_RATE as f64, BLOCK) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let has_atom_in = plugin.ports.iter().any(|p| {
        p.kind == PortKind::AtomSequence && p.direction == PortDirection::Input
    });
    let n_in = instance.n_audio_inputs();
    let n_out = instance.n_audio_outputs();
    println!("audio: {n_in} in / {n_out} out, midi-in: {has_atom_in}");

    let channels = n_out.clamp(1, 2) as u16;
    let mut writer = (n_out > 0).then(|| {
        hound::WavWriter::create(
            &out_path,
            hound::WavSpec {
                channels,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            },
        )
        .expect("cannot create wav file")
    });

    let total_frames = (seconds * SAMPLE_RATE as f64) as usize;
    let n_blocks = total_frames.div_ceil(BLOCK);
    let mut phase: f32 = 0.0;
    let dphase = 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32;
    let mut midi_events_out = 0usize;

    for block in 0..n_blocks {
        let frames = BLOCK.min(total_frames - block * BLOCK);

        // MIDI note on in the first block, note off in the last.
        if has_atom_in {
            if block == 0 {
                instance.push_midi(0, &[0x90, note, 100]).expect("note on");
            }
            if block + 1 == n_blocks {
                instance.push_midi(0, &[0x80, note, 0]).expect("note off");
            }
        }

        // Feed a test sine to any audio inputs (for effect plugins).
        let start_phase = phase;
        for ch in 0..n_in {
            let mut p = start_phase;
            let buf = instance.audio_input_mut(ch).unwrap();
            for s in buf[..frames].iter_mut() {
                *s = 0.5 * p.sin();
                p += dphase;
            }
        }
        phase = start_phase + dphase * frames as f32;

        instance.run(frames).expect("run failed");

        // Count MIDI/atom output events.
        let mut nth = 0;
        while let Some(seq) = instance.atom_output(nth) {
            midi_events_out += seq.events().len();
            nth += 1;
        }

        // Interleave and write audio outputs.
        if let Some(w) = writer.as_mut() {
            let chans: Vec<&[f32]> = (0..channels as usize)
                .map(|c| instance.audio_output(c.min(n_out - 1)).unwrap())
                .collect();
            for i in 0..frames {
                for ch in &chans {
                    w.write_sample(ch[i]).expect("wav write");
                }
            }
        }
    }

    if let Some(w) = writer {
        w.finalize().expect("wav finalize");
        println!("wrote {out_path} ({seconds}s, {channels} ch)");
    }
    if midi_events_out > 0 {
        println!("plugin emitted {midi_events_out} atom events");
    }
    println!("final control values:");
    for (port, value) in instance.controls() {
        println!("  {:<24} = {}", port.symbol, value);
    }
}
