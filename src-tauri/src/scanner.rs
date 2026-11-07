use crate::compression::{inspect, Clip};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

#[derive(Clone, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}
#[derive(Default, Serialize)]
pub struct Scanned {
    pub clips: Vec<Clip>,
    pub errors: Vec<String>,
    pub skipped: Vec<Skipped>,
}

fn app_folder(path: &Path) -> bool {
    path.file_name().is_some_and(|n| {
        n.to_string_lossy()
            .to_lowercase()
            .starts_with("raw compact - ")
    })
}
fn raw_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("nev") || e.eq_ignore_ascii_case("r3d"))
}
fn generated(path: &Path) -> bool {
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    ["_light", "_medium", "_strong"]
        .iter()
        .any(|s| stem.ends_with(s))
        || path.ancestors().skip(1).any(app_folder)
}

fn discover(inputs: &[String], skip: bool) -> (BTreeSet<PathBuf>, Vec<Skipped>, Vec<String>) {
    let mut paths = BTreeSet::new();
    let mut skipped = Vec::new();
    let mut errors = Vec::new();
    for input in inputs {
        let input = Path::new(input);
        if input.is_dir() {
            let walker = WalkDir::new(input)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    let name = entry.file_name().to_string_lossy();
                    if name.starts_with("._") || name.starts_with(".raw-compact-") {
                        return false;
                    }
                    if skip && entry.file_type().is_dir() && app_folder(entry.path()) {
                        skipped.push(Skipped {
                            path: entry.path().to_string_lossy().into_owned(),
                            reason: "RAW Compact output folder".into(),
                        });
                        return false;
                    }
                    true
                });
            for entry in walker {
                match entry {
                    Ok(entry) if entry.file_type().is_file() && raw_file(entry.path()) => {
                        paths.insert(entry.into_path());
                    }
                    Err(e) => errors.push(e.to_string()),
                    _ => {}
                }
            }
        } else if input.is_file() {
            if !input
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("._"))
            {
                paths.insert(input.into());
            }
        } else {
            errors.push(format!("Cannot access {}", input.display()));
        }
    }
    if skip {
        paths.retain(|path| {
            if generated(path) {
                skipped.push(Skipped {
                    path: path.to_string_lossy().into_owned(),
                    reason: "Preset-suffixed copy or app output folder".into(),
                });
                false
            } else {
                true
            }
        });
    }
    (paths, skipped, errors)
}

pub fn scan(paths: &[String], skip: bool) -> Scanned {
    let (paths, mut skipped, errors) = discover(paths, skip);
    skipped.sort_by(|a, b| a.path.cmp(&b.path));
    skipped.dedup_by(|a, b| a.path == b.path);
    let mut result = Scanned {
        clips: Vec::new(),
        errors,
        skipped,
    };
    let mut seen = BTreeSet::new();
    for path in paths {
        match inspect(&path) {
            Ok(clip) if seen.insert(clip.path.clone()) => result.clips.push(clip),
            Ok(_) => {}
            Err(e) => result.errors.push(format!("{}: {e}", path.display())),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recursive_discovery_skips_our_copies_and_irrelevant_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("nested");
        std::fs::create_dir(&sub).unwrap();
        let batch = dir.path().join("RAW Compact - Medium - 123");
        std::fs::create_dir(&batch).unwrap();
        for path in [
            sub.join("CLIP.NEV"),
            sub.join("CLIP_medium.NEV"),
            sub.join("CLIP_light.R3D"),
            sub.join("proxy.MP4"),
            sub.join("._CLIP.NEV"),
            batch.join("CLIP.NEV"),
        ] {
            std::fs::write(path, b"fixture").unwrap();
        }
        let inputs = vec![dir.path().to_string_lossy().into_owned()];
        let (paths, skipped, errors) = discover(&inputs, true);
        assert!(errors.is_empty());
        assert_eq!(paths.len(), 1);
        assert!(paths.contains(&sub.join("CLIP.NEV")));
        assert_eq!(skipped.len(), 3);
        let (all, _, _) = discover(&inputs, false);
        assert_eq!(all.len(), 4);
    }
    #[test]
    fn explicit_generated_file_can_be_included_by_turning_skip_off() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("CLIP_STRONG.nev");
        std::fs::write(&file, b"fixture").unwrap();
        let inputs = vec![file.to_string_lossy().into_owned()];
        assert!(discover(&inputs, true).0.is_empty());
        assert_eq!(discover(&inputs, false).0.len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn recursive_scan_does_not_follow_directory_symlink_loops() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("nested");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("CLIP.NEV"), b"fixture").unwrap();
        std::os::unix::fs::symlink(dir.path(), sub.join("loop")).unwrap();
        assert_eq!(
            discover(&[dir.path().to_string_lossy().into_owned()], true)
                .0
                .len(),
            1
        );
    }
}
