use crate::verification;
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
    pub fn tag(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Medium => "medium",
            Self::Strong => "strong",
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

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    Beside,
    Folder,
}

#[derive(Clone, Serialize)]
pub struct Job {
    pub input: PathBuf,
    pub output: PathBuf,
}
#[derive(Serialize)]
pub struct Batch {
    pub destination: String,
    pub jobs: Vec<Job>,
}

pub fn beside_path(input: &Path, preset: Preset) -> PathBuf {
    let mut name = input.file_stem().unwrap().to_os_string();
    name.push(format!("_{}.", preset.tag()));
    name.push(input.extension().unwrap());
    input.with_file_name(name)
}

pub fn prepare_batch(
    paths: &[String],
    folder: &str,
    mode: OutputMode,
    preset: Preset,
) -> Result<Batch, String> {
    if paths.is_empty() {
        return Err("Add at least one clip.".into());
    }
    let clips = paths
        .iter()
        .map(|p| inspect(Path::new(p)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut names = HashSet::new();
    if mode == OutputMode::Folder {
        for clip in &clips {
            if !names.insert(clip.name.to_lowercase()) {
                return Err("Duplicate clip names: choose ‘Beside each original’ to keep them in their own folders.".into());
            }
        }
    }
    let output_folder = if mode == OutputMode::Folder {
        let folder = Path::new(folder)
            .canonicalize()
            .map_err(|_| "Choose an existing output folder.".to_string())?;
        if !folder.is_dir() {
            return Err("Choose an output folder.".into());
        }
        Some(create_batch_folder(&folder, preset)?)
    } else {
        None
    };
    let destination = output_folder
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Beside each original".into());
    let jobs = clips
        .iter()
        .map(|c| {
            let input = PathBuf::from(&c.path);
            let output = output_folder
                .as_ref()
                .map(|p| p.join(&c.name))
                .unwrap_or_else(|| beside_path(&input, preset));
            Job { input, output }
        })
        .collect();
    Ok(Batch { destination, jobs })
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
    pub verification: String,
    pub original: String,
    pub warning: Option<String>,
}

pub fn publish(
    staged: &Path,
    output: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(f64),
) -> Result<(), String> {
    verification::cancelled(cancel)?;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(staged)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    let temp = tempfile::TempPath::try_from_path(staged).map_err(|e| e.to_string())?;
    match temp.persist_noclobber(output) {
        Ok(()) => Ok(()),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Err(format!(
            "Output already exists; original kept: {}",
            output.display()
        )),
        Err(e) => {
            // Some external filesystems cannot rename without replacement or create hard links.
            // Exclusive creation keeps existing entries safe; verify the copy before accepting it.
            use sha2::{Digest, Sha256};
            use std::io::{Read, Write};
            let mut source = std::fs::File::open(&e.path).map_err(|e| e.to_string())?;
            let size = source.metadata().map_err(|e| e.to_string())?.len();
            let destination = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)
                .map_err(|e| format!("Cannot create a new copy at {}: {e}", output.display()))?;
            let cleanup = tempfile::TempPath::try_from_path(output).map_err(|e| e.to_string())?;
            let expected = (|| -> Result<Vec<u8>, String> {
                let mut destination = destination;
                let mut digest = Sha256::new();
                let mut buf = vec![0; 1024 * 1024];
                let mut written = 0u64;
                loop {
                    verification::cancelled(cancel)?;
                    let n = source.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    destination
                        .write_all(&buf[..n])
                        .map_err(|e| e.to_string())?;
                    digest.update(&buf[..n]);
                    written += n as u64;
                    progress(0.8 * written as f64 / size.max(1) as f64);
                }
                if written != size {
                    return Err("The completed copy has an unexpected size.".into());
                }
                destination.sync_all().map_err(|e| e.to_string())?;
                Ok(digest.finalize().to_vec())
            })()?;
            let actual = verification::hash(output, cancel, &mut |f| progress(0.8 + 0.2 * f))?;
            if expected != actual {
                return Err("Copied bytes failed verification; original kept.".into());
            }
            cleanup.keep().map_err(|e| e.to_string())?;
            Ok(())
        }
    }
}

pub fn compress(
    job: &Job,
    preset: Preset,
    move_to_trash: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(&str, f64, f64),
) -> Result<Completed, String> {
    let input = &job.input;
    let output = &job.output;
    inspect(input)?;
    // symlink_metadata also catches dangling symlinks; never replace any existing entry.
    if std::fs::symlink_metadata(output).is_ok() {
        return Err(format!(
            "Output already exists; original kept: {}",
            output.display()
        ));
    }
    verification::cancelled(cancel)?;
    let original_metadata = std::fs::metadata(input).map_err(|e| e.to_string())?;
    let original_hash = if move_to_trash {
        Some(verification::hash(input, cancel, &mut |f| {
            progress("Checking source", f, 0.0)
        })?)
    } else {
        None
    };
    let staging = tempfile::Builder::new()
        .prefix(".raw-compact-")
        .tempdir_in(output.parent().unwrap())
        .map_err(|e| e.to_string())?;
    let staged = staging.path().join(output.file_name().unwrap());
    let before = bluecompress::probe(input).map_err(|e| e.to_string())?;
    let options = Options {
        input: input.clone(),
        output: staged.clone(),
        quality: preset.quality(),
        bpp: None,
        resolution: Resolution::Original,
        backend: Backend::Auto,
        threads: 0,
        cbr: true,
        template: None,
    };
    let done = bluecompress::transcode(&options, cancel, &mut |f, fps| {
        progress("Compressing", f, fps)
    })
    .map_err(|e| e.to_string())?;
    let after = bluecompress::probe(&staged).map_err(|e| e.to_string())?;
    check_headers(&before, &after)?;
    if move_to_trash {
        verification::full_check(input, &staged, cancel, &mut |f| {
            progress("Verifying every frame", f, 0.0)
        })?;
        let hash = verification::hash(input, cancel, &mut |f| {
            progress("Checking source is unchanged", f, 0.0)
        })?;
        let now = std::fs::metadata(input).map_err(|e| e.to_string())?;
        if original_metadata.len() != now.len()
            || original_metadata.modified().ok() != now.modified().ok()
            || original_hash.as_ref() != Some(&hash)
        {
            return Err("Source changed during processing; original kept.".into());
        }
    }
    verification::cancelled(cancel)?;
    publish(&staged, output, cancel, &mut |f| {
        progress("Saving verified copy", f, 0.0)
    })?;
    let (mut original, mut warning) = ("kept".to_string(), None);
    if move_to_trash && !cancel.load(Ordering::SeqCst) {
        let now = std::fs::metadata(input).map_err(|e| e.to_string())?;
        if original_metadata.len() != now.len()
            || original_metadata.modified().ok() != now.modified().ok()
        {
            return Ok(Completed {
                output: output.to_string_lossy().into_owned(),
                input_bytes: done.input_bytes,
                output_bytes: done.output_bytes,
                seconds: done.seconds,
                backend: done.backend,
                verification: "full".into(),
                original: "kept".into(),
                warning: Some("Source changed after verification; original kept.".into()),
            });
        }
        progress("Moving original to Trash", 1.0, 0.0);
        let mut context = trash::TrashContext::new();
        #[cfg(target_os = "macos")]
        {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            context.set_delete_method(DeleteMethod::NsFileManager);
        }
        match context.delete(input) {
            Ok(()) => original = "trashed".into(),
            Err(e) => {
                original = if input.exists() {
                    "kept"
                } else {
                    "check_trash"
                }
                .into();
                warning = Some(format!(
                    "Compressed copy saved. Could not confirm moving the original to Trash: {e}"
                ));
            }
        }
    }
    Ok(Completed {
        output: output.to_string_lossy().into_owned(),
        input_bytes: done.input_bytes,
        output_bytes: done.output_bytes,
        seconds: done.seconds,
        backend: done.backend,
        verification: if move_to_trash { "full" } else { "basic" }.into(),
        original,
        warning,
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
    #[test]
    fn beside_names_keep_extension_and_parent() {
        assert_eq!(
            beside_path(Path::new("shoot one/clip.NEV"), Preset::Medium),
            Path::new("shoot one/clip_medium.NEV")
        );
        assert_eq!(
            beside_path(Path::new("shoot two/clip.r3d"), Preset::Light),
            Path::new("shoot two/clip_light.r3d")
        );
        assert_ne!(
            beside_path(Path::new("shoot one/clip.NEV"), Preset::Medium),
            beside_path(Path::new("shoot two/clip.NEV"), Preset::Medium)
        );
    }
    #[test]
    fn publishing_never_replaces_existing_file_even_after_preflight() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged.NEV");
        let existing = dir.path().join("existing.NEV");
        std::fs::write(&staged, b"new compressed data").unwrap();
        std::fs::write(&existing, b"original data").unwrap();
        assert!(publish(&staged, &existing, &AtomicBool::new(false), &mut |_| {}).is_err());
        assert_eq!(std::fs::read(existing).unwrap(), b"original data");
    }
    #[test]
    fn new_output_is_published_intact() {
        let dir = tempfile::tempdir().unwrap();
        let staged = dir.path().join("staged.NEV");
        let output = dir.path().join("clip_medium.NEV");
        std::fs::write(&staged, b"verified compressed data").unwrap();
        publish(&staged, &output, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(std::fs::read(output).unwrap(), b"verified compressed data");
    }
    #[test]
    #[ignore = "Requires a disposable copy of a camera clip and native Trash support"]
    fn real_clip_beside_collision_cancel_and_trash() {
        let sample = PathBuf::from(
            std::env::var_os("RAW_COMPACT_TEST_CLIP").expect("set RAW_COMPACT_TEST_CLIP"),
        );
        let root = PathBuf::from(
            std::env::var_os("RAW_COMPACT_TEST_ROOT").expect("set RAW_COMPACT_TEST_ROOT"),
        );
        let scratch = tempfile::Builder::new()
            .prefix("disposable-")
            .tempdir_in(&root)
            .unwrap();
        let cancel = AtomicBool::new(false);
        let original_hash = verification::hash(&sample, &cancel, &mut |_| {}).unwrap();
        let mut paths = Vec::new();
        for name in ["folder one", "folder two"] {
            let dir = scratch.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            let path = dir.join(format!(
                "DISPOSABLE_TEST.{}",
                sample.extension().unwrap().to_string_lossy()
            ));
            std::fs::copy(&sample, &path).unwrap();
            paths.push(path.to_string_lossy().into_owned());
        }
        let batch = prepare_batch(&paths, "", OutputMode::Beside, Preset::Medium).unwrap();
        assert_eq!(batch.jobs.len(), 2);
        for job in &batch.jobs {
            let result = compress(job, Preset::Medium, false, &cancel, &mut |_, _, _| {}).unwrap();
            assert_eq!(result.original, "kept");
            assert!(job.output.is_file());
            assert_eq!(
                verification::hash(&job.input, &cancel, &mut |_| {}).unwrap(),
                original_hash
            );
        }
        let medium_hash = verification::hash(&batch.jobs[0].output, &cancel, &mut |_| {}).unwrap();
        assert!(compress(
            &batch.jobs[0],
            Preset::Medium,
            true,
            &cancel,
            &mut |_, _, _| {}
        )
        .is_err());
        assert_eq!(
            verification::hash(&batch.jobs[0].output, &cancel, &mut |_| {}).unwrap(),
            medium_hash
        );
        assert!(batch.jobs[0].input.exists());
        let input = &batch.jobs[1].input;
        let cancelled_job = Job {
            input: input.clone(),
            output: beside_path(input, Preset::Strong),
        };
        let result = compress(
            &cancelled_job,
            Preset::Strong,
            true,
            &cancel,
            &mut |phase, f, _| {
                if phase == "Verifying every frame" && f > 0.0 {
                    cancel.store(true, Ordering::SeqCst);
                }
            },
        );
        assert!(result.is_err());
        assert!(input.exists());
        assert!(!cancelled_job.output.exists());
        cancel.store(false, Ordering::SeqCst);
        let input = &batch.jobs[0].input;
        let trash_job = Job {
            input: input.clone(),
            output: beside_path(input, Preset::Light),
        };
        let trashed =
            compress(&trash_job, Preset::Light, true, &cancel, &mut |_, _, _| {}).unwrap();
        assert_eq!(trashed.original, "trashed", "{:?}", trashed.warning);
        assert!(!input.exists());
        assert!(trash_job.output.exists());
        assert_eq!(
            verification::hash(&sample, &cancel, &mut |_| {}).unwrap(),
            original_hash
        );
        std::fs::write(
            root.join("validation.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "same_named_clips_in_separate_directories":"passed",
                "keep_originals":"passed","existing_output_preserved":"passed",
                "cancel_during_full_verification_keeps_original":"passed",
                "full_decode_metadata_audio_verification":"passed",
                "trash_disposable_original_only":"passed","camera_source_unchanged":"passed",
                "filesystem":"external SSD"
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
