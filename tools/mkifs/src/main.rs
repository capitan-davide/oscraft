//! `mkifs` - build a QNX OS 8.0 boot image from a buildfile.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use mkifs::image::{Options, build};

const USAGE: &str = "\
usage: mkifs [options] buildfile image
  -v[v...]          verbose
  -r rootdir        search rootdir/<cpu>/{sbin,usr/sbin,boot/sys,bin,usr/bin,lib,lib/dll,usr/lib}
  -l line           process a buildfile line before the buildfile
  -o dir            directory for [+keeplinked] .sym files
  -a suffix         suffix for .sym files
  --linker path     linker for relocatable bootstrap programs (default: ld.lld)
  --link-script f   linker script for them (default: built in)";

fn parse_args() -> Result<Options> {
    let mut args = std::env::args().skip(1);
    let mut opts = Options {
        buildfile: PathBuf::new(),
        output: PathBuf::new(),
        verbose: 0,
        roots: Vec::new(),
        sym_dir: None,
        sym_suffix: None,
        pre_lines: Vec::new(),
        linker: std::env::var_os("MKIFS_LD").map_or_else(|| PathBuf::from("ld.lld"), PathBuf::from),
        link_script: None,
    };
    let mut files = Vec::new();
    while let Some(a) = args.next() {
        // Options take their value attached (-r/dir) or as the next argument.
        let mut value = |opt: &str| -> Result<String> {
            match a.strip_prefix(opt).filter(|v| !v.is_empty()) {
                Some(v) => Ok(v.to_string()),
                None => args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("{opt} needs a value")),
            }
        };
        match a.as_str() {
            "-h" | "--help" | "-?" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--linker" => opts.linker = value("--linker")?.into(),
            "--link-script" => opts.link_script = Some(value("--link-script")?.into()),
            s if s.starts_with("-v") && s[1..].chars().all(|c| c == 'v') => {
                opts.verbose += (s.len() - 1) as u8
            }
            s if s.starts_with("-r") => opts.roots.push(value("-r")?.into()),
            s if s.starts_with("-l") => opts.pre_lines.push(value("-l")?),
            s if s.starts_with("-o") => opts.sym_dir = Some(value("-o")?.into()),
            s if s.starts_with("-a") => opts.sym_suffix = Some(value("-a")?),
            s if s.starts_with('-') && s.len() > 1 => bail!("unknown option {s}\n{USAGE}"),
            _ => files.push(a),
        }
    }
    let [buildfile, output] = files.as_slice() else {
        bail!("{USAGE}")
    };
    opts.buildfile = buildfile.into();
    opts.output = output.into();
    Ok(opts)
}

fn main() -> ExitCode {
    match parse_args().and_then(|o| build(&o)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mkifs: {e:#}");
            ExitCode::FAILURE
        }
    }
}
