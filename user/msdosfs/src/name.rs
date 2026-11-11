//! Names, as `msdosfs_conv.c`: a short 8.3 name for every file, and a long
//! one, in UTF-16 pieces, when the name doesn't fit it.

/// A long name's limit, in UTF-16 units.
pub(crate) const LONG_MAX: usize = 255;
const SHORT_FORBIDDEN: &[u8] = b"\"*+,./:;<=>?[\\]|";

fn short_char(c: char) -> Option<u8> {
    if !c.is_ascii() || c <= ' ' || c == '\x7f' {
        return None;
    }
    let u = c.to_ascii_uppercase() as u8;
    (!SHORT_FORBIDDEN.contains(&u)).then_some(u)
}

/// Whether a name can't be a file's: `msdosfs` refuses these.
pub(crate) fn invalid(name: &str) -> bool {
    name.is_empty()
        || name.chars().any(|c| c < ' ' || "\"*/:<>?\\|".contains(c))
        || name.encode_utf16().count() > LONG_MAX
}

/// The name's exact 8.3 form, when the short name alone says it.
pub(crate) fn exact_short(name: &str) -> Option<[u8; 11]> {
    if name.chars().any(|c| c.is_ascii_lowercase()) {
        return None;
    }
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || base == "." {
        return None;
    }
    let mut out = [b' '; 11];
    for (slot, c) in out.iter_mut().zip(base.chars()) {
        *slot = short_char(c)?;
    }
    for (slot, c) in out[8..].iter_mut().zip(ext.chars()) {
        *slot = short_char(c)?;
    }
    Some(out)
}

/// A short name for a long one, `BASE~n.EXT`.
pub(crate) fn numbered_short(name: &str, n: u32) -> [u8; 11] {
    let (base, ext) = match name.rsplit_once('.') {
        Some((b, e)) if !b.is_empty() => (b, e),
        _ => (name, ""),
    };
    let tail = format!("~{n}");
    let mut out = [b' '; 11];
    let room = 8 - tail.len();
    let mut used = 0;
    for b in base.chars().filter_map(short_char).take(room) {
        out[used] = b;
        used += 1;
    }
    if used == 0 {
        out[0] = b'_';
        used = 1;
    }
    out[used..used + tail.len()].copy_from_slice(tail.as_bytes());
    for (slot, b) in out[8..].iter_mut().zip(ext.chars().filter_map(short_char)) {
        *slot = b;
    }
    out
}

/// The 8.3 name as text, with lower-case flags (NT's bits 3 and 4).
pub(crate) fn short_text(raw: &[u8], flags: u8) -> String {
    let piece = |b: &[u8], lower: bool| {
        let t: String = b.iter().map(|&c| if c < 0x80 { c as char } else { '_' }).collect();
        let t = t.trim_end().to_string();
        if lower { t.to_ascii_lowercase() } else { t }
    };
    let mut first = [raw[0]; 8];
    first[1..].copy_from_slice(&raw[1..8]);
    if first[0] == 0x05 {
        first[0] = 0xe5;
    }
    let base = piece(&first, flags & 8 != 0);
    let ext = piece(&raw[8..11], flags & 16 != 0);
    if ext.is_empty() { base } else { format!("{base}.{ext}") }
}

/// A long entry's checksum of the short name.
pub(crate) fn checksum(short: &[u8]) -> u8 {
    short[..11].iter().fold(0u8, |s, &b| s.rotate_right(1).wrapping_add(b))
}

/// Whether two names are the same file: case isn't significant.
pub(crate) fn same(a: &str, b: &str) -> bool {
    a.chars().flat_map(char::to_lowercase).eq(b.chars().flat_map(char::to_lowercase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_fit_or_not() {
        assert_eq!(&exact_short("README.TXT").unwrap(), b"README  TXT");
        assert_eq!(&exact_short("A").unwrap(), b"A          ");
        assert!(exact_short("readme.txt").is_none(), "case would be lost");
        assert!(exact_short("LONGERNAME.TXT").is_none());
        assert!(exact_short("A+B.TXT").is_none());
    }

    #[test]
    fn numbered() {
        assert_eq!(&numbered_short("a long name.text", 1), b"ALONGN~1TEX");
        assert_eq!(&numbered_short(".hidden", 12), b"HIDDE~12   ");
        assert_eq!(&numbered_short("é", 1), b"_~1        ");
    }

    #[test]
    fn text_and_flags() {
        assert_eq!(short_text(b"README  TXT", 0), "README.TXT");
        assert_eq!(short_text(b"README  TXT", 8 | 16), "readme.txt");
        assert_eq!(short_text(b"BIN        ", 0), "BIN");
    }

    #[test]
    fn the_checksum_of_a_known_name() {
        // Worked by hand with the algorithm of the FAT specification.
        let mut s = 0u8;
        for &b in b"FOO     BAR" {
            s = (s >> 1 | s << 7).wrapping_add(b);
        }
        assert_eq!(checksum(b"FOO     BAR"), s);
    }

    #[test]
    fn names_refused_and_equal() {
        assert!(invalid("a/b") && invalid("a?") && invalid(""));
        assert!(!invalid("a b.c"));
        assert!(same("ReadMe.TXT", "readme.txt"));
        assert!(same("Ä", "ä"));
    }
}
