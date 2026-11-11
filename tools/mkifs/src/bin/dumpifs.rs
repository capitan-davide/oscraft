//! `dumpifs` - describe a QNX OS 8.0 boot image.

use std::process::ExitCode;

use mkifs::dump::Image;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbose = args.iter().any(|a| a == "-v");
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let [path] = files.as_slice() else {
        eprintln!("usage: dumpifs [-v] image");
        return ExitCode::FAILURE;
    };
    let result = std::fs::read(path)
        .map_err(anyhow::Error::from)
        .and_then(|data| Image::parse(&data).map(|i| i.describe(verbose)));
    match result {
        Ok(text) => {
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("dumpifs: {path}: {e:#}");
            ExitCode::FAILURE
        }
    }
}
