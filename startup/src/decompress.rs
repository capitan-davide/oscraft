//! Decompressing a compressed image file system.
//!
//! mkifs can compress the image file system (not startup, which must run
//! first): with `+compress`, startup's header says which algorithm, and
//! the image file system is a series of blocks, each a big-endian 16-bit
//! length and that many bytes of compressed data, decompressing to at
//! most 64 KB, until a length of 0. QNX's startup programs decompress
//! UCL's NRV2B ("UCL8", mkifs's default), and so does this one. (mkifs can
//! also make LZO and zlib images, which QNX's startup programs do not
//! boot either.) Each block is compressed on its own, so the decompressor
//! needs no state between blocks.
//!
//! NRV2B is an LZ77 variant: the stream mixes literal bytes with
//! *matches*, copies of earlier output given as a distance back and a
//! length.

/// Why a stream could not be decompressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The input ended in the middle of a block.
    InputOverrun,
    /// The output would not fit.
    OutputOverrun,
    /// A match points before the start of the output.
    LookBehind,
}

/// The compression algorithms startup supports (`STARTUP_HDR_FLAGS1_COMPRESS_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Ucl,
}

impl Algorithm {
    /// The algorithm of a startup header's `flags1`, if supported.
    pub fn from_flags1(flags1: u8) -> Option<Self> {
        match flags1 & crate::abi::startup::STARTUP_HDR_FLAGS1_COMPRESS_MASK {
            0x0c => Some(Algorithm::Ucl),
            _ => None,
        }
    }
}

/// Decompress the blocks at the start of `src` into `dst`. Returns the
/// number of bytes written.
pub fn decompress(algorithm: Algorithm, src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let (mut ip, mut op) = (0, 0);
    loop {
        let len = match src.get(ip..ip + 2) {
            Some(b) => usize::from(b[0]) << 8 | usize::from(b[1]),
            None => return Err(Error::InputOverrun),
        };
        ip += 2;
        if len == 0 {
            return Ok(op);
        }
        let block = src.get(ip..ip + len).ok_or(Error::InputOverrun)?;
        op += match algorithm {
            Algorithm::Ucl => nrv2b(block, &mut dst[op..])?,
        };
        ip += len;
    }
}

/// Output with bounds checks, and copies of earlier output.
struct Out<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl Out<'_> {
    fn push(&mut self, b: u8) -> Result<(), Error> {
        *self.buf.get_mut(self.pos).ok_or(Error::OutputOverrun)? = b;
        self.pos += 1;
        Ok(())
    }

    /// Copy `len` bytes from `dist` bytes back, a byte at a time: the copy
    /// may overlap what it writes (a run of the same bytes).
    fn copy_match(&mut self, dist: usize, len: usize) -> Result<(), Error> {
        let from = self.pos.checked_sub(dist).ok_or(Error::LookBehind)?;
        if self.pos + len > self.buf.len() {
            return Err(Error::OutputOverrun);
        }
        for i in 0..len {
            self.buf[self.pos + i] = self.buf[from + i];
        }
        self.pos += len;
        Ok(())
    }
}

/// Input with bounds checks.
struct In<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl In<'_> {
    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.buf.get(self.pos).ok_or(Error::InputOverrun)?;
        self.pos += 1;
        Ok(b)
    }
}

/// UCL's NRV2B, 8-bit variant. Control bits come from bytes of the input,
/// most significant first, interleaved with the data bytes: a 1 bit means
/// a literal byte follows; a 0 bit starts a match, whose offset and length
/// are encoded in variable-length bit sequences (*gamma codes*: pairs of
/// a data bit and a "more" bit).
fn nrv2b(src: &[u8], dst: &mut [u8]) -> Result<usize, Error> {
    let mut input = In { buf: src, pos: 0 };
    let mut out = Out { buf: dst, pos: 0 };
    // The bit buffer: a byte shifted left, with a 1 marking its end.
    let mut bb: u32 = 0;
    let mut bit = |input: &mut In| -> Result<u32, Error> {
        bb = if bb & 0x7f != 0 {
            bb * 2
        } else {
            u32::from(input.byte()?) * 2 + 1
        };
        Ok((bb >> 8) & 1)
    };
    let mut last_off: u32 = 1;
    loop {
        while bit(&mut input)? == 1 {
            let b = input.byte()?;
            out.push(b)?;
        }
        let mut off: u32 = 1;
        loop {
            off = off * 2 + bit(&mut input)?;
            if bit(&mut input)? == 1 {
                break;
            }
        }
        if off == 2 {
            off = last_off;
        } else {
            off = (off - 3)
                .wrapping_mul(256)
                .wrapping_add(u32::from(input.byte()?));
            if off == u32::MAX {
                return Ok(out.pos);
            }
            off += 1;
            last_off = off;
        }
        let mut len = bit(&mut input)?;
        len = len * 2 + bit(&mut input)?;
        if len == 0 {
            len = 1;
            loop {
                len = len * 2 + bit(&mut input)?;
                if bit(&mut input)? == 1 {
                    break;
                }
            }
            len += 2;
        }
        len += u32::from(off > 0xd00);
        out.copy_match(off as usize, len as usize + 1)?;
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::vec;

    /// FNV-1a, to compare outputs without storing them.
    fn fnv(data: &[u8]) -> u64 {
        data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        })
    }

    // The last two blocks of an image file system compressed by mkifs, with
    // the length and hash of what the SDP's dumpifs decompressed them to.
    #[test]
    fn decompresses_mkifs_ucl_blocks() {
        let mut out = vec![0; 0x20000];
        let n = decompress(Algorithm::Ucl, include_bytes!("testdata/ucl.bin"), &mut out).unwrap();
        assert_eq!((n, fnv(&out[..n])), (67380, 0x41db_3154_9f4c_bad7));
    }

    #[test]
    fn reports_short_input_and_output() {
        let mut small = vec![0; 100];
        let data = include_bytes!("testdata/ucl.bin");
        assert_eq!(
            decompress(Algorithm::Ucl, data, &mut small),
            Err(Error::OutputOverrun)
        );
        let mut out = vec![0; 0x20000];
        assert_eq!(
            decompress(Algorithm::Ucl, &data[..100], &mut out),
            Err(Error::InputOverrun)
        );
    }
}
