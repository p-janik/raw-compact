#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod compression;
mod verification;
use compression::{Clip, OutputMode, Preset};
use serde::Serialize;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct Runner {
    active: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    quitting: Arc<AtomicBool>,
}

#[tauri::command]
async fn choose_clips(app: tauri::AppHandle) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Nikon RAW", &["r3d", "R3D", "nev", "NEV"])
            .blocking_pick_files()
            .unwrap_or_default()
            .into_iter()
            .map(|p| {
                p.into_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .map_err(|e| e.to_string())
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn choose_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|p| {
                p.into_path()
                    .map(|p| p.to_string_lossy().into_owned())
                    .map_err(|e| e.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
struct Scanned {
    clips: Vec<Clip>,
    errors: Vec<String>,
}
#[tauri::command]
async fn scan(paths: Vec<String>) -> Result<Scanned, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut result = Scanned {
            clips: vec![],
            errors: vec![],
        };
        for path in paths {
            match compression::inspect(Path::new(&path)) {
                Ok(clip) => result.clips.push(clip),
                Err(e) => result.errors.push(format!("{path}: {e}")),
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel(state: State<Runner>) {
    state.cancel.store(true, Ordering::SeqCst);
}

#[derive(Clone, Serialize)]
struct Progress {
    path: String,
    fraction: f64,
    fps: f64,
    phase: String,
}
#[derive(Clone, Serialize)]
struct JobError {
    path: String,
    error: String,
    cancelled: bool,
}

#[tauri::command]
async fn start(
    app: tauri::AppHandle,
    state: State<'_, Runner>,
    paths: Vec<String>,
    folder: String,
    preset: Preset,
    mode: OutputMode,
    move_to_trash: bool,
) -> Result<(), String> {
    if state.active.swap(true, Ordering::SeqCst) {
        return Err("A batch is already running.".into());
    }
    state.cancel.store(false, Ordering::SeqCst);
    let active = state.active.clone();
    let cancel = state.cancel.clone();
    let quitting = state.quitting.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let plan = compression::prepare_batch(&paths, &folder, mode, preset);
        match plan {
            Err(error) => {
                let _ = app.emit("batch-error", error);
            }
            Ok(batch) => {
                let _ = app.emit("batch-started", &batch);
                for job in batch.jobs {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let path = job.input.to_string_lossy().into_owned();
                    let _ = app.emit("job-started", &path);
                    let mut last_progress = Instant::now();
                    let mut last_phase = String::new();
                    let result = compression::compress(
                        &job,
                        preset,
                        move_to_trash,
                        &cancel,
                        &mut |phase, fraction, fps| {
                            if phase == last_phase
                                && fraction < 1.0
                                && last_progress.elapsed() < Duration::from_millis(100)
                            {
                                return;
                            }
                            last_phase = phase.into();
                            last_progress = Instant::now();
                            let _ = app.emit(
                                "job-progress",
                                Progress {
                                    path: path.clone(),
                                    fraction,
                                    fps,
                                    phase: phase.into(),
                                },
                            );
                        },
                    );
                    match result {
                        Ok(done) => {
                            let _ = app
                                .emit("job-done", serde_json::json!({"path":path,"result":done}));
                        }
                        Err(error) => {
                            let _ = app.emit(
                                "job-error",
                                JobError {
                                    path,
                                    error,
                                    cancelled: cancel.load(Ordering::SeqCst),
                                },
                            );
                        }
                    }
                }
            }
        }
        active.store(false, Ordering::SeqCst);
        let _ = app.emit("batch-finished", ());
        if quitting.load(Ordering::SeqCst) {
            app.exit(0);
        }
    });
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Runner::default())
        .invoke_handler(tauri::generate_handler![
            choose_clips,
            choose_folder,
            scan,
            start,
            cancel
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<Runner>();
                if state.active.load(Ordering::SeqCst) {
                    api.prevent_close();
                    state.quitting.store(true, Ordering::SeqCst);
                    state.cancel.store(true, Ordering::SeqCst);
                    let _ = window.emit("closing", ());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Could not start RAW Compact")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                let state = app.state::<Runner>();
                if state.active.load(Ordering::SeqCst) {
                    api.prevent_exit();
                    state.quitting.store(true, Ordering::SeqCst);
                    state.cancel.store(true, Ordering::SeqCst);
                }
            }
        });
}
