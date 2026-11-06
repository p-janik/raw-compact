use bluecompress::{Backend, ClipInfo, Codec, Options, Resolution};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Light,
    Medium,
    Strong,
}
impl Preset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Medium => "Medium",
            Self::Strong => "Strong",
        }
    }
    pub fn quality(self) -> u8 {
        match self {
            Self::Light => 2,
            Self::Medium => 3,
            Self::Strong => 4,
        }
    }
}

#[derive(Clone, Serialize)]
pub struct Clip {
    pub path: String,
    pub name: String,
    pub parent: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub fps: f64,
    pub bytes: u64,
    pub bpp: f64,
}

pub fn inspect(path: &Path) -> Result<Clip, String> {
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    let ext = path
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or_default();
    if !ext.eq_ignore_ascii_case("r3d") && !ext.eq_ignore_ascii_case("nev") {
        return Err("Choose Nikon R3D NE (.R3D) or N-RAW (.NEV) clips.".into());
    }
    let i = bluecompress::probe(&path).map_err(|e| e.to_string())?;
    let format = match i.codec {
        Codec::R3dNe => "R3D NE",
        Codec::NRaw => "N-RAW",
        _ => return Err("Only Nikon R3D NE and N-RAW are supported.".into()),
    };
    if i.frames == 0 {
        return Err("The clip has no video frames.".into());
    }
    Ok(Clip {
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        parent: path.parent().unwrap().to_string_lossy().into_owned(),
        path: path.to_string_lossy().into_owned(),
        format: format.into(),
        width: i.width,
        height: i.height,
        frames: i.frames,
        fps: i.fps,
        bytes: i.file_bytes,
        bpp: i.bpp,
    })
}

pub fn validate_batch(paths: &[String], folder: &str) -> Result<PathBuf, String> {
    if paths.is_empty() {
        return Err("Add at least one clip.".into());
    }
    let folder = Path::new(folder)
        .canonicalize()
        .map_err(|_| "Choose an existing output folder.".to_string())?;
    if !folder.is_dir() {
        return Err("Choose an output folder.".into());
    }
    let mut names = HashSet::new();
    for path in paths {
        let clip = inspect(Path::new(path))?;
        if !names.insert(clip.name.to_lowercase()) {
            return Err(format!(
                "{} appears more than once. Use separate batches for clips with the same name.",
                clip.name
            ));
        }
    }
    Ok(folder)
}

pub fn create_batch_folder(folder: &Path, preset: Preset) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let path = folder.join(format!("RAW Compact - {} - {stamp}", preset.label()));
    std::fs::create_dir(&path).map_err(|e| format!("Cannot create a batch folder: {e}"))?;
    Ok(path)
}

pub fn check_headers(a: &ClipInfo, b: &ClipInfo) -> Result<(), String> {
    if a.codec != b.codec
        || a.width != b.width
        || a.height != b.height
        || a.frames != b.frames
        || (a.fps - b.fps).abs() > 0.000001
        || a.audio_tracks != b.audio_tracks
    {
        return Err(
            "Output verification failed: format, resolution, timing or audio track count changed."
                .into(),
        );
    }
    Ok(())
}

#[derive(Serialize)]
pub struct Completed {
    pub output: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub seconds: f64,
    pub backend: String,
}

pub fn compress(
    input: &Path,
    folder: &Path,
    preset: Preset,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64, f64),
) -> Result<Completed, String> {
    let clip = inspect(input)?;
    let output = folder.join(&clip.name);
    if output.exists()
        || output
            .with_file_name(format!("{}.partial", clip.name))
            .exists()
    {
        return Err(format!("Output already exists: {}", output.display()));
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("Cancelled".into());
    }
    let before = bluecompress::probe(input).map_err(|e| e.to_string())?;
    let options = Options {
        input: input.into(),
        output: output.clone(),
        quality: preset.quality(),
        bpp: None,
        resolution: Resolution::Original,
        backend: Backend::Auto,
        threads: 0,
        cbr: true,
        template: None,
    };
    let done = bluecompress::transcode(&options, cancel, progress).map_err(|e| e.to_string())?;
    let checked = bluecompress::probe(&output)
        .map_err(|e| e.to_string())
        .and_then(|after| check_headers(&before, &after));
    if let Err(e) = checked {
        // Keep a failed verification visibly separate from a usable RAW copy.
        let quarantine = output.with_file_name(format!("{}.unverified", clip.name));
        let moved = std::fs::rename(&output, &quarantine);
        return Err(match moved {
            Ok(()) => format!("{e} Saved as {}", quarantine.display()),
            Err(_) => format!("{e} Do not use {}", output.display()),
        });
    }
    Ok(Completed {
        output: output.to_string_lossy().into_owned(),
        input_bytes: done.input_bytes,
        output_bytes: done.output_bytes,
        seconds: done.seconds,
        backend: done.backend,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn info() -> ClipInfo {
        ClipInfo {
            codec: Codec::R3dNe,
            width: 6060,
            height: 3410,
            frames: 561,
            fps: 59.94,
            file_bytes: 100,
            video_bytes: 90,
            codestream_bytes: 80,
            audio_tracks: 1,
            bpp: 2.0,
        }
    }
    #[test]
    fn header_check_accepts_size_change_but_rejects_lost_frames_or_audio() {
        let original = info();
        let mut output = info();
        output.file_bytes = 55;
        assert!(check_headers(&original, &output).is_ok());
        output.frames -= 1;
        assert!(check_headers(&original, &output).is_err());
        output = info();
        output.audio_tracks = 0;
        assert!(check_headers(&original, &output).is_err());
        output = info();
        output.codec = Codec::NRaw;
        assert!(check_headers(&original, &output).is_err());
    }
}
