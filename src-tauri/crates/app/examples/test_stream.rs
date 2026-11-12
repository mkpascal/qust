// A local live stream for measurements and manual tests: a WAV tone in real time on
// 127.0.0.1, optionally with an ICY title. Prints the port, then serves until killed.
//
//   cargo run -p qust-app --example test_stream -- [--name <station>] [--title <ICY title>]
//   cargo run -p qust-app --example test_stream -- --write-tone <file.wav> <seconds>

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    if let Some(path) = value("--write-tone") {
        let seconds = args.last().and_then(|s| s.parse().ok()).unwrap_or(120.0);
        if let Err(error) = qust_ui::diag::write_tone_wav(std::path::Path::new(&path), seconds) {
            eprintln!("test_stream: {error}");
            std::process::exit(1);
        }
        return;
    }
    let name = value("--name").unwrap_or_else(|| "Test Radio".into());
    let title = value("--title");
    match qust_app::test_stream::serve(&name, title.as_deref()) {
        Ok(port) => {
            println!("{port}");
            loop {
                std::thread::park();
            }
        }
        Err(error) => {
            eprintln!("test_stream: {error}");
            std::process::exit(1);
        }
    }
}
