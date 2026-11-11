//! `diskd-md`: a memory disk or a disk image, served: `-f image` or
//! `-s megabytes`, as `/dev/md<unit>`, mounted at `-m path` if given.

use std::fs::OpenOptions;

use bufcache::BlockDevice;
use md::{Image, Memory};

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut file, mut size, mut mount, mut unit) = (None, 8usize, None, 0u32);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-f" => file = args.next(),
            "-s" => size = args.next().and_then(|s| s.parse().ok()).unwrap_or(size),
            "-m" => mount = args.next(),
            "-u" => unit = args.next().and_then(|u| u.parse().ok()).unwrap_or(unit),
            _ => {
                eprintln!("usage: diskd-md [-f image | -s megabytes] [-u unit] [-m path]");
                std::process::exit(2);
            }
        }
    }
    let device: Box<dyn BlockDevice + Send> = match file {
        Some(path) => match OpenOptions::new().read(true).write(true).open(&path).and_then(Image::new) {
            Ok(i) => Box::new(i),
            Err(e) => {
                eprintln!("diskd-md: {path}: {e}");
                std::process::exit(1);
            }
        },
        None => Box::new(Memory::new(size << 20)),
    };
    diskd::run(device, &format!("/dev/md{unit}"), mount.as_deref());
}
