use std::{
    fs,
    path::{Path, PathBuf},
};

use egui::{self, Ui};
use egui_graph_edit::{GraphEditorOptions, NodeResponse, NodeTemplateTrait};

use crate::animation_graph::{AnimGraphEditor, AnimGraphResponse, AnimNodeTemplate};
#[cfg(feature = "standalone")]
use crate::{inspector, preview::PreviewState};
#[cfg(feature = "standalone")]
use bevy::{
    gltf::Gltf,
    prelude::{AnimationGraph, Assets},
};

#[cfg(feature = "standalone")]
pub struct EmbeddedPreview<'a> {
    pub state: &'a mut PreviewState,
    pub gltfs: &'a Assets<Gltf>,
    pub graphs: &'a Assets<AnimationGraph>,
}

#[cfg(not(feature = "standalone"))]
pub struct EmbeddedPreview<'a>(std::marker::PhantomData<&'a ()>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum FileAction {
    OpenProject,
    SaveProjectAs,
    ImportGlb,
    ExportRuntime,
}

impl FileAction {
    fn title(self) -> &'static str {
        match self {
            Self::OpenProject => "Open animation graph project",
            Self::SaveProjectAs => "Save animation graph project",
            Self::ImportGlb => "Import GLB",
            Self::ExportRuntime => "Export runtime graph",
        }
    }
    fn is_save(self) -> bool {
        matches!(self, Self::SaveProjectAs | Self::ExportRuntime)
    }
}

/// Graph authoring panel for egui hosts. The standalone Bevy app uses the
/// same [`AnimGraphEditor`] model and provides the 3D preview.
#[derive(Default)]
pub struct EmbeddedAnimGraphEditor {
    pub editor: AnimGraphEditor,
    gltf_asset_path: Option<String>,
    file_action: Option<FileAction>,
    file_path: String,
    file_filter: String,
    file_candidates: Vec<PathBuf>,
}

impl EmbeddedAnimGraphEditor {
    /// Asset-relative GLB path currently displayed by the editor.
    pub fn preview_asset_path(&self) -> Option<&str> {
        self.gltf_asset_path.as_deref()
    }
    /// `assets_root` is the host project's asset folder. External GLBs are
    /// copied into `imports/` so project paths remain usable by Bevy.
    pub fn ui(&mut self, ui: &mut Ui, assets_root: Option<&Path>) {
        self.ui_impl(ui, assets_root, None);
    }

    #[cfg(feature = "standalone")]
    pub fn ui_with_preview(
        &mut self,
        ui: &mut Ui,
        assets_root: Option<&Path>,
        preview: EmbeddedPreview<'_>,
    ) {
        self.ui_impl(ui, assets_root, Some(preview));
    }

    #[allow(unused_variables, unused_mut)]
    fn ui_impl(
        &mut self,
        ui: &mut Ui,
        assets_root: Option<&Path>,
        mut preview: Option<EmbeddedPreview<'_>>,
    ) {
        self.editor.sanitize_after_graph_change();
        ui.horizontal_wrapped(|ui| {
            if ui.button("New").clicked() {
                self.new_project();
            }
            if ui.button("Open…").clicked() {
                self.begin_file_action(FileAction::OpenProject, assets_root);
            }
            if ui.button("Save").clicked() {
                if let Some(path) = self.editor.current_project_path.clone() {
                    if let Err(error) = self.write_project(&path) {
                        self.editor.last_event = format!("Save failed: {error}");
                    }
                } else {
                    self.begin_file_action(FileAction::SaveProjectAs, assets_root);
                }
            }
            if ui.button("Save As…").clicked() {
                self.begin_file_action(FileAction::SaveProjectAs, assets_root);
            }
            if ui.button("Import GLB…").clicked() {
                self.begin_file_action(FileAction::ImportGlb, assets_root);
            }
            if ui.button("Export Runtime…").clicked() {
                self.begin_file_action(FileAction::ExportRuntime, assets_root);
            }
            if ui.button("Reset View").clicked() {
                self.editor.graph.reset_zoom(ui);
                self.editor.graph.pan_zoom.pan = egui::Vec2::ZERO;
            }
        });
        ui.label(&self.editor.last_event);
        ui.separator();

        let available = ui.available_size();
        let inspector_width = 250.0_f32.min((available.x * 0.35).max(180.0));
        let orbiting = ui.ctx().input(|input| input.modifiers.shift);
        ui.horizontal(|ui| {
            let graph_width =
                (available.x - inspector_width - ui.spacing().item_spacing.x).max(1.0);
            ui.allocate_ui_with_layout(
                egui::vec2(graph_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    // Node widgets can mutate their values directly, even when the
                    // graph editor ignores its own selection/drag responses. Disable
                    // the whole graph UI while the camera owns Shift gestures.
                    ui.add_enabled_ui(!orbiting, |ui| self.graph_ui(ui, !orbiting));
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(inspector_width, available.y),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    #[cfg(feature = "standalone")]
                    if let Some(preview) = preview.as_mut() {
                        egui::ScrollArea::vertical()
                            .id_salt("embedded_anim_graph_inspector")
                            .show(ui, |ui| {
                                inspector::draw_inspector(
                                    ui,
                                    &mut self.editor,
                                    Some(&mut *preview.state),
                                    preview.gltfs,
                                    preview.graphs,
                                );
                            });
                        return;
                    }
                    self.inspector_ui(ui);
                },
            );
        });
        self.file_action_ui(ui.ctx(), assets_root);
    }

    fn new_project(&mut self) {
        self.editor = AnimGraphEditor::default();
        let output = AnimNodeTemplate::Output;
        let node = self.editor.graph.graph.add_node(
            output.node_graph_label(&mut self.editor.ui_state),
            output.user_data(&mut self.editor.ui_state),
            |graph, id| output.build_node(graph, &mut self.editor.ui_state, id),
        );
        self.editor
            .graph
            .node_positions
            .insert(node, egui::pos2(80.0, 80.0));
        self.editor.graph.node_order.push(node);
        self.editor.preview_output = Some(node);
        self.editor.last_event =
            "New graph created. Right-click the canvas to add nodes.".to_owned();
        self.gltf_asset_path = None;
        self.file_action = None;
        self.file_candidates.clear();
    }

    fn graph_ui(&mut self, ui: &mut Ui, interactions_enabled: bool) {
        self.editor.ui_state.preview_output = self.editor.preview_output;
        self.editor.ui_state.one_shot_action_clip_labels =
            self.editor.one_shot_action_clip_labels();
        let response = self.editor.graph.draw_graph_editor_with_options(
            ui,
            self.editor.templates,
            &mut self.editor.ui_state,
            Vec::new(),
            GraphEditorOptions {
                interactions_enabled,
            },
        );
        for event in response.node_responses {
            match event {
                NodeResponse::CreatedNode(_) => {
                    self.editor.ensure_playback_inputs();
                    self.editor.last_event = "Node created".to_owned();
                }
                NodeResponse::ConnectEventEnded { .. } => {
                    self.editor.last_event = "Connection added".to_owned();
                }
                NodeResponse::DeleteNodeFull { node, .. } => {
                    self.editor.sanitize_after_graph_change();
                    self.editor.last_event = format!("Deleted {}", node.label);
                }
                NodeResponse::User(AnimGraphResponse::SetOutput(node)) => {
                    self.editor.preview_output = Some(node);
                    self.editor.sanitize_after_graph_change();
                    self.editor.last_event = "Output selected".to_owned();
                }
                NodeResponse::User(AnimGraphResponse::AddOneShotLane(node)) => {
                    if self.editor.add_one_shot_lane(node) {
                        self.editor.sanitize_after_graph_change();
                        self.editor.last_event = "One shot lane added".to_owned();
                    }
                }
                NodeResponse::User(AnimGraphResponse::RemoveOneShotLane(node)) => {
                    if self.editor.remove_one_shot_lane(node) {
                        self.editor.sanitize_after_graph_change();
                        self.editor.last_event = "One shot lane removed".to_owned();
                    }
                }
                _ => {}
            }
        }
        self.editor.sync_node_labels();
    }

    fn inspector_ui(&mut self, ui: &mut Ui) {
        ui.heading("Inspector");
        ui.separator();
        ui.label(format!("Nodes: {}", self.editor.graph.graph.nodes.len()));
        ui.label(format!(
            "Connections: {}",
            self.editor.graph.graph.connections.len()
        ));
        if let Some(path) = self.editor.current_project_path.as_deref() {
            ui.label(format!("Project: {}", path.display()));
        }
        if let Some(gltf) = self.gltf_asset_path.as_deref() {
            ui.label(format!("GLB: {gltf}"));
            ui.label(format!(
                "Animation clips: {}",
                self.editor.ui_state.available_clips.len()
            ));
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for clip in &self.editor.ui_state.available_clips {
                        ui.label(clip);
                    }
                });
            ui.weak("Shift+drag to orbit the 3D preview.");
        }
        if let Some(node) = self
            .editor
            .graph
            .selected_nodes
            .first()
            .and_then(|id| self.editor.graph.graph.nodes.get(*id))
        {
            ui.separator();
            ui.heading(&node.label);
            ui.label(format!("{:?}", node.user_data.template));
            ui.label(&node.user_data.note);
        } else {
            ui.separator();
            ui.label("No node selected");
        }
    }

    fn begin_file_action(&mut self, action: FileAction, assets_root: Option<&Path>) {
        self.file_path = match action {
            FileAction::OpenProject => self
                .editor
                .current_project_path
                .as_deref()
                .map_or_else(String::new, |path| path.display().to_string()),
            FileAction::SaveProjectAs => self
                .editor
                .current_project_path
                .clone()
                .or_else(|| assets_root.map(|root| root.join("graphs/new.animgraph_project.ron")))
                .map_or_else(String::new, |path| path.display().to_string()),
            FileAction::ImportGlb => assets_root
                .map(|root| root.display().to_string())
                .unwrap_or_default(),
            FileAction::ExportRuntime => assets_root
                .map(|root| root.join("graphs/new.animgraph_runtime.ron"))
                .map_or_else(String::new, |path| path.display().to_string()),
        };
        self.file_filter.clear();
        self.file_candidates = if action.is_save() {
            Vec::new()
        } else {
            assets_root.map_or_else(Vec::new, |root| {
                let extension = if action == FileAction::ImportGlb {
                    "glb"
                } else {
                    "ron"
                };
                let mut files = asset_files(root, extension);
                if action == FileAction::OpenProject {
                    files.retain(|path| {
                        path.file_name()
                            .is_some_and(|name| name.to_string_lossy().contains("animgraph"))
                    });
                }
                files
            })
        };
        self.file_action = Some(action);
    }

    fn file_action_ui(&mut self, ctx: &egui::Context, assets_root: Option<&Path>) {
        let Some(action) = self.file_action else {
            return;
        };
        let mut open = true;
        let mut selected_path = None;
        egui::Window::new(action.title())
            .id(egui::Id::new("embedded_anim_graph_file_action"))
            .collapsible(false)
            .resizable(true)
            .default_width(580.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Path");
                    let entry = ui.text_edit_singleline(&mut self.file_path);
                    if entry.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                        selected_path = Some(PathBuf::from(self.file_path.trim()));
                    }
                    if ui.button("Browse…").clicked() {
                        let dialog = rfd::FileDialog::new()
                            .set_title(action.title())
                            .set_directory(assets_root.unwrap_or_else(|| Path::new(".")));
                        let picked = match action {
                            FileAction::OpenProject => {
                                dialog.add_filter("RON", &["ron"]).pick_file()
                            }
                            FileAction::ImportGlb => dialog.add_filter("GLB", &["glb"]).pick_file(),
                            _ => dialog.add_filter("RON", &["ron"]).save_file(),
                        };
                        if let Some(path) = picked {
                            self.file_path = path.display().to_string();
                        }
                    }
                });
                if !action.is_save() {
                    ui.horizontal(|ui| {
                        ui.label("Filter");
                        ui.text_edit_singleline(&mut self.file_filter);
                    });
                    if let Some(root) = assets_root {
                        egui::ScrollArea::vertical()
                            .max_height(240.0)
                            .show(ui, |ui| {
                                for path in self.file_candidates.iter().filter(|path| {
                                    path.to_string_lossy()
                                        .to_lowercase()
                                        .contains(&self.file_filter.to_lowercase())
                                }) {
                                    let label = path
                                        .strip_prefix(root)
                                        .unwrap_or(path)
                                        .display()
                                        .to_string();
                                    let response = ui.selectable_label(
                                        self.file_path == path.display().to_string(),
                                        label,
                                    );
                                    if response.clicked() {
                                        self.file_path = path.display().to_string();
                                    }
                                    if response.double_clicked() {
                                        selected_path = Some(path.clone());
                                    }
                                }
                            });
                    }
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(match action {
                            FileAction::OpenProject => "Open project",
                            FileAction::SaveProjectAs => "Save project",
                            FileAction::ImportGlb => "Import GLB",
                            FileAction::ExportRuntime => "Export runtime graph",
                        })
                        .clicked()
                    {
                        selected_path = Some(PathBuf::from(self.file_path.trim()));
                    }
                    if ui.button("Cancel").clicked() {
                        self.file_action = None;
                    }
                });
            });
        if !open {
            self.file_action = None;
        }
        if let Some(path) = selected_path {
            if path.as_os_str().is_empty() || path.is_dir() {
                self.editor.last_event = "Select a file path.".to_owned();
                return;
            }
            let result = match action {
                FileAction::OpenProject => self.open_project(&path, assets_root),
                FileAction::SaveProjectAs => self.write_project(&path),
                FileAction::ImportGlb => self.import_glb(&path, assets_root),
                FileAction::ExportRuntime => self.export_runtime(&path),
            };
            match result {
                Ok(()) => self.file_action = None,
                Err(error) => {
                    self.editor.last_event = format!("{} failed: {error}", action.title())
                }
            }
        }
    }

    fn open_project(&mut self, path: &Path, assets_root: Option<&Path>) -> Result<(), String> {
        let gltf_path = self
            .editor
            .load_from_path(path)
            .map_err(|error| error.to_string())?;
        self.gltf_asset_path = gltf_path;
        let warning = self.refresh_clips(assets_root);
        self.editor.last_event = match warning {
            Some(warning) => format!("Loaded {} ({warning})", path.display()),
            None => format!("Loaded {}", path.display()),
        };
        Ok(())
    }

    fn write_project(&mut self, path: &Path) -> Result<(), String> {
        self.editor
            .save_to_path(path, self.gltf_asset_path.as_deref())
            .map_err(|error| error.to_string())?;
        self.editor.current_project_path = Some(path.to_path_buf());
        self.editor.last_event = format!("Saved {}", path.display());
        Ok(())
    }

    fn export_runtime(&mut self, path: &Path) -> Result<(), String> {
        self.editor
            .save_runtime_graph_to_path(path, self.gltf_asset_path.as_deref())
            .map_err(|error| error.to_string())?;
        self.editor.last_event = format!("Exported {}", path.display());
        Ok(())
    }

    fn import_glb(&mut self, source: &Path, assets_root: Option<&Path>) -> Result<(), String> {
        let root = assets_root.ok_or("Set the launcher project root before importing a GLB")?;
        let source = source.canonicalize().map_err(|error| error.to_string())?;
        if !source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
        {
            return Err("Only .glb files are supported in the embedded editor".to_owned());
        }
        let clips = animation_names(&source)?;
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let asset = if let Ok(relative) = source.strip_prefix(&root) {
            relative.to_path_buf()
        } else {
            let file_name = source.file_name().ok_or("GLB has no file name")?;
            let destination = root.join("imports").join(file_name);
            if destination.exists() && destination.canonicalize().ok().as_ref() != Some(&source) {
                return Err(format!(
                    "{} already exists; choose a different file name",
                    destination.display()
                ));
            }
            fs::create_dir_all(root.join("imports")).map_err(|error| error.to_string())?;
            if destination.canonicalize().ok().as_ref() != Some(&source) {
                fs::copy(&source, &destination).map_err(|error| error.to_string())?;
            }
            PathBuf::from("imports").join(file_name)
        };
        let asset = asset.to_string_lossy().replace('\\', "/");
        self.gltf_asset_path = Some(asset.clone());
        self.editor.ui_state.available_clips = clips;
        self.editor.last_event = format!(
            "Imported {asset} ({} animation clips)",
            self.editor.ui_state.available_clips.len()
        );
        Ok(())
    }

    fn refresh_clips(&mut self, assets_root: Option<&Path>) -> Option<String> {
        self.editor.ui_state.available_clips.clear();
        let (Some(root), Some(asset)) = (assets_root, self.gltf_asset_path.as_deref()) else {
            return None;
        };
        match animation_names(&root.join(asset)) {
            Ok(clips) => {
                self.editor.ui_state.available_clips = clips;
                None
            }
            Err(error) => Some(format!("GLB clips unavailable: {error}")),
        }
    }
}

fn animation_names(path: &Path) -> Result<Vec<String>, String> {
    let gltf = gltf::Gltf::open(path).map_err(|error| error.to_string())?;
    Ok(gltf
        .animations()
        .enumerate()
        .map(|(index, animation)| {
            animation
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Animation {index}"))
        })
        .collect())
}

fn asset_files(root: &Path, extension: &str) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("anim_graph_{tag}_{}_{}", std::process::id(), nonce));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn new_project_clears_previous_project_and_clip_selection() {
        let mut panel = EmbeddedAnimGraphEditor::default();
        panel.editor.current_project_path = Some(PathBuf::from("old.ron"));
        panel.gltf_asset_path = Some("imports/old.glb".to_owned());
        panel
            .editor
            .ui_state
            .available_clips
            .push("Idle".to_owned());
        panel.new_project();
        assert!(panel.editor.current_project_path.is_none());
        assert!(panel.gltf_asset_path.is_none());
        assert!(panel.editor.ui_state.available_clips.is_empty());
        assert_eq!(panel.editor.graph.graph.nodes.len(), 1);
        assert!(panel.editor.last_event.contains("New graph"));
    }

    #[test]
    fn saved_project_opens_with_its_glb_reference() {
        let root = temp_dir("project");
        let path = root.join("example.animgraph_project.ron");
        let mut panel = EmbeddedAnimGraphEditor::default();
        panel.gltf_asset_path = Some("characters/example.glb".to_owned());
        panel.write_project(&path).unwrap();
        panel.new_project();
        panel.open_project(&path, None).unwrap();

        assert_eq!(
            panel.editor.current_project_path.as_deref(),
            Some(path.as_path())
        );
        assert_eq!(
            panel.gltf_asset_path.as_deref(),
            Some("characters/example.glb")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn glb_import_exposes_named_animation_clips() {
        let root = temp_dir("glb");
        let path = root.join("animated.glb");
        let mut json = br#"{"asset":{"version":"2.0"},"animations":[{"name":"Idle","channels":[],"samplers":[]}]}"#.to_vec();
        while json.len() % 4 != 0 {
            json.push(b' ');
        }
        let total_length = (12 + 8 + json.len()) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"glTF");
        bytes.extend_from_slice(&2_u32.to_le_bytes());
        bytes.extend_from_slice(&total_length.to_le_bytes());
        bytes.extend_from_slice(&(json.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"JSON");
        bytes.extend_from_slice(&json);
        fs::write(&path, bytes).unwrap();

        let mut panel = EmbeddedAnimGraphEditor::default();
        panel.import_glb(&path, Some(&root)).unwrap();
        assert_eq!(panel.gltf_asset_path.as_deref(), Some("animated.glb"));
        assert_eq!(panel.editor.ui_state.available_clips, ["Idle"]);
        fs::remove_dir_all(root).unwrap();
    }
}
