use bluecompress::{
    bmff::{fourcc, InputFile, Movie, Sample},
    develop::decode_planes,
    nraw::NrawFrame,
    tico::TicoHeader,
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err("Cancelled; original kept.".into())
    } else {
        Ok(())
    }
}

pub fn hash(
    path: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<Vec<u8>, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let size = f.metadata().map_err(|e| e.to_string())?.len().max(1);
    let mut h = Sha256::new();
    let mut buf = vec![0; 1024 * 1024];
    let mut read = 0u64;
    loop {
        cancelled(cancel)?;
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        read += n as u64;
        progress(read as f64 / size as f64);
    }
    Ok(h.finalize().to_vec())
}

// Compare all movie metadata except sample byte sizes and physical offsets.
fn metadata(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < data.len() {
        if data.len() - at < 8 {
            return Err("Truncated movie metadata.".into());
        }
        let size32 = u32::from_be_bytes(data[at..at + 4].try_into().unwrap());
        let kind = &data[at + 4..at + 8];
        let (size, header) = if size32 == 1 {
            if data.len() - at < 16 {
                return Err("Truncated metadata box.".into());
            }
            (
                usize::try_from(u64::from_be_bytes(
                    data[at + 8..at + 16].try_into().unwrap(),
                ))
                .map_err(|e| e.to_string())?,
                16,
            )
        } else if size32 == 0 {
            (data.len() - at, 8)
        } else {
            (size32 as usize, 8)
        };
        if size < header || size > data.len() - at {
            return Err("Invalid metadata box size.".into());
        }
        if ![b"stsz".as_slice(), b"stco", b"co64"].contains(&kind) {
            out.extend_from_slice(kind);
            if [
                b"moov".as_slice(),
                b"trak",
                b"mdia",
                b"minf",
                b"stbl",
                b"dinf",
                b"edts",
            ]
            .contains(&kind)
            {
                out.extend(metadata(&data[at + header..at + size])?);
            } else {
                out.extend_from_slice(&((size - header) as u64).to_be_bytes());
                out.extend_from_slice(&data[at + header..at + size]);
            }
            out.extend_from_slice(b"/end");
        }
        at += size;
    }
    Ok(out)
}

fn frame_metadata(input: &InputFile, sample: Sample) -> Result<(Vec<u8>, Vec<u8>), String> {
    let data = input
        .read(sample.offset, sample.size as usize)
        .map_err(|e| e.to_string())?;
    let mut nf = NrawFrame::parse(&data).map_err(|e| e.to_string())?;
    if nf.header.len() < 4 {
        return Err("Truncated RAW frame header.".into());
    }
    nf.header[..4].fill(0);
    if let Some(at) = nf.nrti_offset {
        nf.header[at..at + 4].fill(0);
    }
    let (mut h, _) = TicoHeader::parse(&data[nf.codestream_offset..]).map_err(|e| e.to_string())?;
    h.raw[h.lcod_offset..h.lcod_offset + 4].fill(0);
    Ok((nf.header, h.raw))
}

pub fn full_check(
    source: &Path,
    output: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<(), String> {
    cancelled(cancel)?;
    let a = InputFile::open(source).map_err(|e| e.to_string())?;
    let b = InputFile::open(output).map_err(|e| e.to_string())?;
    let ma = Movie::parse(&a).map_err(|e| e.to_string())?;
    let mb = Movie::parse(&b).map_err(|e| e.to_string())?;
    if ma.tracks.len() != mb.tracks.len() || metadata(&ma.moov)? != metadata(&mb.moov)? {
        return Err("Movie metadata or timing changed; original kept.".into());
    }
    let nodes = |m: &Movie| {
        m.top
            .iter()
            .filter(|n| ![fourcc(b"moov"), fourcc(b"mdat"), fourcc(b"free")].contains(&n.kind))
            .cloned()
            .collect::<Vec<_>>()
    };
    let (na, nb) = (nodes(&ma), nodes(&mb));
    if na.len() != nb.len() {
        return Err("Container metadata count changed.".into());
    }
    for (x, y) in na.iter().zip(&nb) {
        cancelled(cancel)?;
        if x.kind != y.kind
            || a.read(x.payload(), (x.size - x.header_size as u64) as usize)
                .map_err(|e| e.to_string())?
                != b.read(y.payload(), (y.size - y.header_size as u64) as usize)
                    .map_err(|e| e.to_string())?
        {
            return Err("Container metadata changed.".into());
        }
    }
    for (ta, tb) in ma.tracks.iter().zip(&mb.tracks) {
        if ta.samples.len() != tb.samples.len() {
            return Err("Track sample count changed.".into());
        }
        if ta.handler != fourcc(b"vide") {
            for (x, y) in ta.samples.iter().zip(&tb.samples) {
                cancelled(cancel)?;
                if a.read(x.offset, x.size as usize)
                    .map_err(|e| e.to_string())?
                    != b.read(y.offset, y.size as usize)
                        .map_err(|e| e.to_string())?
                {
                    return Err("Audio or another non-video sample changed.".into());
                }
            }
        }
    }
    let frames = mb.video().samples.len();
    if frames == 0 || frames != ma.video().samples.len() {
        return Err("Video frame count changed.".into());
    }
    for (i, (x, y)) in ma
        .video()
        .samples
        .iter()
        .zip(&mb.video().samples)
        .enumerate()
    {
        cancelled(cancel)?;
        if frame_metadata(&a, *x)? != frame_metadata(&b, *y)? {
            return Err(format!("RAW metadata changed in frame {}.", i + 1));
        }
        decode_planes(&b, &mb, i)
            .map_err(|e| format!("Cannot decode output frame {}: {e}", i + 1))?;
        progress((i + 1) as f64 / frames as f64);
    }
    cancelled(cancel)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn corrupt_metadata_is_rejected_without_panicking() {
        assert!(metadata(&[0, 0, 0, 50, b'm', b'o', b'o', b'v']).is_err());
        assert!(metadata(&[1, 2, 3]).is_err());
    }
    #[test]
    fn cancellation_prevents_verification_and_removal() {
        let cancel = AtomicBool::new(true);
        assert!(full_check(
            Path::new("missing.NEV"),
            Path::new("copy.NEV"),
            &cancel,
            &mut |_| {}
        )
        .is_err());
    }
}
