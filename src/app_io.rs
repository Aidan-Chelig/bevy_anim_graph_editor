use std::{fs, path::Path};

use bevy::prelude::*;
use bevy_anim_graph_editor::{animation_graph::AnimGraphEditor, runtime};
use bevy_egui::egui;

use crate::{PreviewState, StartupInput, preview};

pub(crate) fn apply_startup_input(
    startup_input: Res<StartupInput>,
    mut editor: ResMut<AnimGraphEditor>,
    mut preview: ResMut<PreviewState>,
    asset_server: Res<AssetServer>,
) {
    let Some(path) = startup_input.path.as_deref() else {
        return;
    };

    match startup_path_kind(path) {
        Some(StartupPathKind::Project) => match editor.load_from_path(path) {
            Ok(gltf_asset_path) => {
                editor.last_event = format!("Loaded {}", path.display());
                preview.last_applied_signature = None;
                if let Some(gltf_asset_path) = gltf_asset_path {
                    if preview::asset_path_exists(&gltf_asset_path) {
                        preview.load_asset_path(gltf_asset_path, &asset_server);
                    } else {
                        preview.clear_asset(format!("Missing GLB: assets/{gltf_asset_path}"));
                    }
                } else {
                    preview.clear_asset("Project has no GLB path");
                }
            }
            Err(error) => {
                editor.last_event = format!("Startup project load failed: {error}");
            }
        },
        Some(StartupPathKind::Gltf) => match preview.import_gltf(path, &asset_server) {
            Ok(()) => editor.last_event = format!("Imported {}", path.display()),
            Err(error) => editor.last_event = format!("Startup GLB import failed: {error}"),
        },
        None => {
            editor.last_event = format!("Unsupported startup file: {}", path.display());
        }
    }
}

enum StartupPathKind {
    Project,
    Gltf,
}

fn startup_path_kind(path: &Path) -> Option<StartupPathKind> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();

    match extension.as_str() {
        "ron" => Some(StartupPathKind::Project),
        "glb" | "gltf" => Some(StartupPathKind::Gltf),
        _ => None,
    }
}

pub(crate) fn save_project(
    editor: &mut AnimGraphEditor,
    preview: Option<&PreviewState>,
    ui: &mut egui::Ui,
) {
    if let Some(path) = editor.current_project_path.clone() {
        write_project(editor, preview, &path, ui);
    } else {
        save_project_as(editor, preview, ui);
    }
}

pub(crate) fn save_project_as(
    editor: &mut AnimGraphEditor,
    preview: Option<&PreviewState>,
    ui: &mut egui::Ui,
) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Save Animation Graph Project")
        .add_filter("Animation graph project", &["ron"])
        .set_file_name("default.animgraph_project.ron")
        .save_file()
    {
        write_project(editor, preview, &path, ui);
    }
}

fn write_project(
    editor: &mut AnimGraphEditor,
    preview: Option<&PreviewState>,
    path: &Path,
    ui: &mut egui::Ui,
) {
    let gltf_asset_path = preview_gltf_asset_path(preview);
    match editor.save_to_path(path, gltf_asset_path) {
        Ok(()) => {
            editor.current_project_path = Some(path.to_path_buf());
            editor.last_event = format!("Saved {}", path.display());
            ui.close();
        }
        Err(error) => editor.last_event = format!("Save failed: {error}"),
    }
}

pub(crate) fn export_bevy_animation_graph(
    editor: &mut AnimGraphEditor,
    preview: Option<&PreviewState>,
    gltfs: &Assets<bevy::gltf::Gltf>,
    ui: &mut egui::Ui,
) {
    let Some(preview) = preview else {
        editor.last_event = "Bevy export failed: preview not initialized".to_string();
        return;
    };
    let Some(gltf) = preview::loaded_gltf(preview, gltfs) else {
        editor.last_event = "Bevy export failed: GLB is not loaded yet".to_string();
        return;
    };
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export Bevy Animation Graph")
        .add_filter("Bevy animation graph", &["animgraph.ron", "ron"])
        .set_file_name("default.animgraph.ron")
        .save_file()
    else {
        return;
    };

    let compiled = match runtime::compile_editor_graph(editor, gltf) {
        Ok(compiled) => compiled,
        Err(error) => {
            editor.last_event = format!("Bevy export failed: {error}");
            return;
        }
    };

    let result = (|| {
        let mut serialized = String::new();
        compiled.graph.save(&mut serialized)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, serialized)?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })();

    match result {
        Ok(()) => {
            editor.last_event = format!("Exported Bevy graph {}", path.display());
            ui.close();
        }
        Err(error) => editor.last_event = format!("Bevy export failed: {error}"),
    }
}

pub(crate) fn export_runtime_graph(
    editor: &mut AnimGraphEditor,
    preview: Option<&PreviewState>,
    ui: &mut egui::Ui,
) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Export Runtime Animation Graph")
        .add_filter("Runtime animation graph", &["ron"])
        .set_file_name("default.animgraph_runtime.ron")
        .save_file()
    {
        match editor.save_runtime_graph_to_path(&path, preview_gltf_asset_path(preview)) {
            Ok(()) => {
                editor.last_event = format!("Exported runtime graph {}", path.display());
                ui.close();
            }
            Err(error) => editor.last_event = format!("Runtime export failed: {error}"),
        }
    }
}

fn preview_gltf_asset_path(preview: Option<&PreviewState>) -> Option<&str> {
    preview.and_then(|preview| preview.asset_path.as_deref())
}

pub(crate) fn load_project(
    editor: &mut AnimGraphEditor,
    preview: Option<&mut PreviewState>,
    asset_server: &AssetServer,
) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Load Animation Graph Project")
        .add_filter("Animation graph project", &["ron"])
        .pick_file()
    {
        match editor.load_from_path(&path) {
            Ok(gltf_asset_path) => {
                let mut load_message = format!("Loaded {}", path.display());
                if let Some(preview) = preview {
                    preview.last_applied_signature = None;
                    if let Some(gltf_asset_path) = gltf_asset_path {
                        if preview::asset_path_exists(&gltf_asset_path) {
                            preview.load_asset_path(gltf_asset_path, asset_server);
                        } else if let Some(relinked_path) = rfd::FileDialog::new()
                            .set_title("Relink Missing GLB")
                            .add_filter("glTF", &["glb", "gltf"])
                            .pick_file()
                        {
                            match preview.import_gltf(&relinked_path, asset_server) {
                                Ok(()) => {
                                    load_message = format!(
                                        "Loaded {} and relinked {}",
                                        path.display(),
                                        relinked_path.display()
                                    );
                                }
                                Err(error) => {
                                    preview.clear_asset(format!("Relink failed: {error}"));
                                    load_message = format!("Relink failed: {error}");
                                }
                            }
                        } else {
                            preview.clear_asset(format!("Missing GLB: assets/{gltf_asset_path}"));
                        }
                    } else {
                        preview.clear_asset("Project has no GLB path");
                    }
                }
                editor.last_event = load_message;
            }
            Err(error) => editor.last_event = format!("Load failed: {error}"),
        }
    }
}

pub(crate) fn import_gltf(
    editor: &mut AnimGraphEditor,
    preview: Option<&mut PreviewState>,
    asset_server: &AssetServer,
) {
    if let Some(path) = rfd::FileDialog::new()
        .set_title("Import GLB")
        .add_filter("glTF", &["glb", "gltf"])
        .pick_file()
    {
        if let Some(preview) = preview {
            match preview.import_gltf(&path, asset_server) {
                Ok(()) => editor.last_event = format!("Imported {}", path.display()),
                Err(error) => editor.last_event = format!("Import failed: {error}"),
            }
        } else {
            editor.last_event = "Preview not initialized".to_string();
        }
    }
}

pub(crate) fn apply_graph(editor: &mut AnimGraphEditor, preview: Option<&mut PreviewState>) {
    if let Some(preview) = preview {
        preview.apply_requested = true;
        editor.last_event = "Applying graph to preview".to_string();
    } else {
        editor.last_event = "Preview not initialized".to_string();
    }
}
