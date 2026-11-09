//! `cargo run --release -p mc-voice --example transcribe -- <model.bin> <audio.pcm> [language]`
//! Audio: raw 16 kHz mono s16le, e.g. `ffmpeg -i in.wav -f s16le -ar 16000 -ac 1 out.pcm`.

fn main() {
    let mut args = std::env::args().skip(1);
    let model = args.next().expect("model path");
    let audio = args.next().expect("pcm path");
    let language = args.next().unwrap_or_default();
    let pcm = std::fs::read(&audio).expect("read audio");
    let t = std::time::Instant::now();
    let whisper = mc_voice::Whisper::load(&model).expect("load");
    eprintln!("loaded in {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let text = whisper.transcribe(&pcm, &language, "").expect("transcribe");
    eprintln!("recognized in {:?}", t.elapsed());
    println!("{text}");
}
