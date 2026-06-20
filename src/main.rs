use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use bevy::audio::AudioPlugin;
use bevy::prelude::*;
use bevy::winit::WinitSettings;
use bevy_anim_graph_editor::{
    animation_graph::{
        AnimGraphEditor, AnimGraphResponse, EdgeVisualization, connection_contribution_value,
        connection_marker_value,
    },
    runtime,
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use egui_graph_edit::{ConnectionRenderInfo, NodeResponse};

mod preview;

use preview::PreviewState;

#[derive(Resource, Default)]
struct StartupInput {
    path: Option<PathBuf>,
}

fn main() {
    let startup_input = StartupInput {
        path: std::env::args_os().nth(1).map(PathBuf::from),
    };

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Bevy Animation Graph Editor".to_string(),
                        ..default()
                    }),
                    ..default()
                })
                .disable::<AudioPlugin>(),
        )
        .add_plugins(EguiPlugin::default())
        .insert_resource(WinitSettings::desktop_app())
        .insert_resource(startup_input)
        .init_resource::<AnimGraphEditor>()
        .add_systems(
            Startup,
            (preview::setup_preview_scene, apply_startup_input).chain(),
        )
        .add_systems(
            Update,
            (
                preview::build_preview_animation_graph,
                preview::reload_preview_scene,
                preview::update_preview_diagnostics,
                preview::attach_preview_animation_graph,
                preview::apply_editor_graph_to_preview,
                preview::sync_editor_graph_to_preview,
                preview::control_preview_camera,
                preview::cycle_preview_animation,
                preview::toggle_preview_playback,
            ),
        )
        .add_systems(EguiPrimaryContextPass, editor_ui)
        .run();
}

fn editor_ui(
    mut contexts: EguiContexts,
    mut editor: ResMut<AnimGraphEditor>,
    mut preview: Option<ResMut<PreviewState>>,
    gltfs: Res<Assets<bevy::gltf::Gltf>>,
    graphs: Res<Assets<AnimationGraph>>,
    asset_server: Res<AssetServer>,
    time: Res<Time>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    ctx.set_visuals(egui::Visuals::dark());
    suppress_egui_mouse_when_shift_held(ctx);
    let editor = editor.as_mut();
    let preview_playback_active = preview
        .as_deref()
        .is_some_and(|preview| preview.playback_active);
    if preview_playback_active {
        ctx.request_repaint_after(Duration::from_secs_f32(1.0 / 30.0));
    }

    egui::TopBottomPanel::top("main_menu").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Save").clicked() {
                    save_project(editor, preview.as_deref(), ui);
                }
                if ui.button("Save As...").clicked() {
                    save_project_as(editor, preview.as_deref(), ui);
                }
                ui.menu_button("Export", |ui| {
                    if ui.button("Bevy Animation Graph...").clicked() {
                        export_bevy_animation_graph(editor, preview.as_deref(), &gltfs, ui);
                    }
                    if ui.button("Runtime Graph...").clicked() {
                        export_runtime_graph(editor, preview.as_deref(), ui);
                    }
                });
                ui.separator();
                if ui.button("Load Project").clicked() {
                    load_project(editor, preview.as_deref_mut(), &asset_server);
                    ui.close();
                }
                ui.separator();
                if ui.button("Import GLB").clicked() {
                    import_gltf(editor, preview.as_deref_mut(), &asset_server);
                    ui.close();
                }
            });

            ui.menu_button("Edit", |ui| {
                if ui.button("Apply Graph").clicked() {
                    apply_graph(editor, preview.as_deref_mut());
                    ui.close();
                }
                if ui.button("Reset View").clicked() {
                    editor.graph.reset_zoom(ui);
                    editor.graph.pan_zoom.pan = egui::Vec2::ZERO;
                    ui.close();
                }
            });

            ui.menu_button("About", |ui| {
                ui.label("Bevy Animation Graph Editor");
                ui.label("egui-graph-edit prototype");
                ui.separator();
                ui.label("Space toggles playback");
                ui.label("Enter cycles animation");
                ui.label("Shift + Left Drag orbits preview");
                ui.label("Shift + Right Drag pans preview");
                ui.label("Shift + Scroll zooms preview");
            });

            ui.separator();
            ui.strong("Animation Graph");
            ui.separator();
            ui.label(&editor.last_event);
            if let Some(preview) = preview.as_deref() {
                ui.separator();
                ui.label(&preview.status);
            }
        });
    });

    egui::SidePanel::right("inspector")
        .resizable(true)
        .default_width(260.0)
        .show(ctx, |ui| {
            ui.heading("Inspector");
            ui.separator();
            ui.label(format!("Nodes: {}", editor.graph.graph.nodes.len()));
            ui.label(format!(
                "Connections: {}",
                editor.graph.graph.connections.len()
            ));

            ui.separator();
            ui.heading("Visualization");
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut editor.ui_state.edge_visualization,
                    EdgeVisualization::Marker,
                    "Marker",
                );
                ui.selectable_value(
                    &mut editor.ui_state.edge_visualization,
                    EdgeVisualization::Flow,
                    "Flow",
                );
            });
            ui.checkbox(
                &mut editor.ui_state.weight_header_saturation,
                "Weight header saturation",
            );
            ui.checkbox(
                &mut editor.ui_state.contribution_borders,
                "Contribution borders",
            );

            ui.separator();
            ui.heading("Preview");
            if let Some(preview) = preview.as_deref_mut() {
                ui.label(&preview.status);
                ui.label(format!("Scenes: {}", preview.scene_count));
                ui.label(format!("Animations: {}", preview.animations.len()));
                ui.label(format!("Live clips: {}", preview.live_clips.len()));
                ui.label(format!("Players: {}", preview.player_count));
                ui.checkbox(&mut preview.auto_apply, "Auto apply graph");
                ui.label(if preview.last_applied_signature.is_some() {
                    "Applied graph: current or watching"
                } else {
                    "Applied graph: raw GLB clips"
                });
                if let Some(name) = preview.animation_names.get(preview.active_animation) {
                    ui.label(format!("Active: {name}"));
                }
                if let Some(gltf) = preview::loaded_gltf(preview, &gltfs) {
                    let validation = preview::validate_editor_graph(editor, gltf);
                    ui.separator();
                    ui.heading(if validation.can_apply {
                        "Validation"
                    } else {
                        "Validation errors"
                    });
                    if validation.can_apply {
                        ui.label(validation.message);
                    } else {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 140, 110),
                            validation.message,
                        );
                    }
                }
                ui.separator();
                ui.label("Space toggles playback");
                ui.label("Enter cycles animation");

                ui.separator();
                ui.heading("Native Bevy Tree");
                if let Some(graph_handle) = preview.graph.as_ref() {
                    if let Some(graph) = graphs.get(graph_handle) {
                        egui::ScrollArea::vertical()
                            .id_salt("native_bevy_tree_scroll")
                            .max_height(220.0)
                            .show(ui, |ui| {
                                for line in
                                    runtime::native_tree_lines(graph, &preview.native_node_names)
                                {
                                    ui.monospace(line);
                                }
                            });
                    } else {
                        ui.label("Graph asset is loading");
                    }
                } else {
                    ui.label("No Bevy graph asset");
                }
            } else {
                ui.label("Preview not initialized");
            }

            if let Some(output) = editor.preview_output {
                let label = editor
                    .graph
                    .graph
                    .nodes
                    .get(output)
                    .map(|node| node.label.as_str())
                    .unwrap_or("Missing output");
                ui.label(format!("Output: {label}"));
            } else {
                ui.label("Output: sample graph");
            }

            if let Some(node_id) = editor.graph.selected_nodes.first().copied() {
                let node = &editor.graph.graph.nodes[node_id];
                ui.separator();
                ui.heading(&node.label);
                ui.label(format!("{:?}", node.user_data.template));
                ui.label(&node.user_data.note);
            } else {
                ui.separator();
                ui.label("No node selected");
            }
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            editor.ui_state.preview_output = editor.preview_output;
            editor.ui_state.live_one_shot_progress = preview
                .as_deref()
                .map(|preview_state| {
                    preview_state
                        .live_one_shots
                        .iter()
                        .map(|one_shot| (one_shot.editor_node, one_shot.progress))
                        .collect()
                })
                .unwrap_or_default();
            editor.ui_state.available_clips = preview
                .as_deref()
                .and_then(|preview_state| preview::loaded_gltf(preview_state, &gltfs))
                .map(preview::clip_names)
                .unwrap_or_default();
            let response = editor.graph.draw_graph_editor(
                ui,
                editor.templates,
                &mut editor.ui_state,
                Vec::new(),
            );
            draw_connection_visualization(ui, editor, &response.connections, time.elapsed_secs());

            for event in response.node_responses {
                match event {
                    NodeResponse::CreatedNode(_) => {
                        editor.ensure_playback_inputs();
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = "Node created".to_string();
                    }
                    NodeResponse::ConnectEventEnded { .. } => {
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = "Connection added".to_string();
                    }
                    NodeResponse::DeleteNodeFull { node, .. } => {
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = format!("Deleted {}", node.label);
                    }
                    NodeResponse::User(AnimGraphResponse::SetOutput(node_id)) => {
                        editor.preview_output = Some(node_id);
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = "Preview output selected".to_string();
                    }
                    _ => {}
                }
            }
            editor.sync_node_labels();
        });

    Ok(())
}

fn suppress_egui_mouse_when_shift_held(ctx: &egui::Context) {
    if !ctx.input(|input| input.modifiers.shift) {
        return;
    }

    ctx.input_mut(|input| {
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::PointerMoved(_)
                    | egui::Event::MouseMoved(_)
                    | egui::Event::PointerButton { .. }
                    | egui::Event::PointerGone
                    | egui::Event::Touch { .. }
                    | egui::Event::MouseWheel { .. }
                    | egui::Event::Zoom(_)
                    | egui::Event::Rotate(_)
            )
        });
        input.raw_scroll_delta = egui::Vec2::ZERO;
        input.smooth_scroll_delta = egui::Vec2::ZERO;
    });
}

fn apply_startup_input(
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
                if let Some(gltf_asset_path) = gltf_asset_path {
                    if preview::asset_path_exists(&gltf_asset_path) {
                        preview.load_asset_path(gltf_asset_path, &asset_server);
                    } else {
                        preview.status = format!("Missing GLB: assets/{gltf_asset_path}");
                    }
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

fn save_project(editor: &mut AnimGraphEditor, preview: Option<&PreviewState>, ui: &mut egui::Ui) {
    if let Some(path) = editor.current_project_path.clone() {
        write_project(editor, preview, &path, ui);
    } else {
        save_project_as(editor, preview, ui);
    }
}

fn save_project_as(
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

fn export_bevy_animation_graph(
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

fn export_runtime_graph(
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

fn load_project(
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
                                Err(error) => load_message = format!("Relink failed: {error}"),
                            }
                        } else {
                            preview.status = format!("Missing GLB: assets/{gltf_asset_path}");
                        }
                    }
                }
                editor.last_event = load_message;
            }
            Err(error) => editor.last_event = format!("Load failed: {error}"),
        }
    }
}

fn import_gltf(
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

fn apply_graph(editor: &mut AnimGraphEditor, preview: Option<&mut PreviewState>) {
    if let Some(preview) = preview {
        preview.apply_requested = true;
        editor.last_event = "Applying graph to preview".to_string();
    } else {
        editor.last_event = "Preview not initialized".to_string();
    }
}

fn draw_connection_visualization(
    ui: &egui::Ui,
    editor: &mut AnimGraphEditor,
    connections: &[ConnectionRenderInfo],
    elapsed_secs: f32,
) {
    if matches!(editor.ui_state.edge_visualization, EdgeVisualization::Flow) {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f32(1.0 / 30.0));
    }

    let delta_secs = editor
        .ui_state
        .flow_last_time
        .map(|last_time| (elapsed_secs - last_time).clamp(0.0, 0.1))
        .unwrap_or(0.0);
    editor.ui_state.flow_last_time = Some(elapsed_secs);

    let painter = ui.painter();
    for connection in connections {
        if editor.ui_state.contribution_borders {
            draw_connection_contribution_border(
                painter,
                connection,
                connection_contribution_value(
                    &editor.graph.graph,
                    connection.input,
                    editor.preview_output,
                    &editor.ui_state.live_one_shot_progress,
                ),
            );
        }

        let Some(value) = connection_marker_value(
            &editor.graph.graph,
            connection.input,
            connection.output,
            &editor.ui_state.live_one_shot_progress,
        ) else {
            continue;
        };

        match editor.ui_state.edge_visualization {
            EdgeVisualization::Marker => draw_connection_marker(painter, connection.points, value),
            EdgeVisualization::Flow => {
                let phase = editor
                    .ui_state
                    .flow_phases
                    .entry((connection.input, connection.output))
                    .or_insert(0.0);
                draw_connection_flow(painter, connection.points, value, phase, delta_secs);
            }
        }
    }
}

fn draw_connection_contribution_border(
    painter: &egui::Painter,
    connection: &ConnectionRenderInfo,
    contribution: f32,
) {
    let contribution = contribution.clamp(0.0, 1.0);
    if contribution <= 0.001 {
        return;
    }

    let alpha = (35.0 + contribution * 185.0) as u8;
    let glow = egui::epaint::CubicBezierShape::from_points_stroke(
        connection.points,
        false,
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(
            12.0,
            egui::Color32::from_rgba_unmultiplied(126, 236, 255, alpha),
        ),
    );
    painter.add(glow);

    let foreground = egui::epaint::CubicBezierShape::from_points_stroke(
        connection.points,
        false,
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(5.0, connection.color),
    );
    painter.add(foreground);
}

fn draw_connection_marker(painter: &egui::Painter, points: [egui::Pos2; 4], value: f32) {
    let position = cubic_bezier_point(points, value);
    painter.circle_filled(position, 5.0, egui::Color32::from_rgb(245, 176, 75));
    painter.circle_stroke(
        position,
        6.0,
        egui::Stroke::new(1.25, egui::Color32::from_rgb(25, 28, 31)),
    );
}

fn draw_connection_flow(
    painter: &egui::Painter,
    points: [egui::Pos2; 4],
    value: f32,
    phase: &mut f32,
    delta_secs: f32,
) {
    let value = value.clamp(0.0, 1.0);

    const FLOW_DOT_SLOTS: usize = 10;

    let curve = SampledBezier::new(points);
    if curve.length <= f32::EPSILON {
        return;
    }
    let speed_pixels_per_second = 18.0 + value * 95.0;
    *phase = (*phase + delta_secs * speed_pixels_per_second / curve.length).fract();

    let alpha = (35.0 + value * 220.0) as u8;
    if alpha == 0 {
        return;
    }
    let radius = 2.5 + value * 2.0;
    let color = egui::Color32::from_rgba_unmultiplied(245, 176, 75, alpha);
    let outline = egui::Color32::from_rgba_unmultiplied(25, 28, 31, alpha.saturating_sub(25));

    for index in 0..FLOW_DOT_SLOTS {
        let spacing = index as f32 / FLOW_DOT_SLOTS as f32;
        let distance_fraction = (*phase + spacing).fract();
        let position = curve.point_at_distance_fraction(distance_fraction);
        painter.circle_filled(position, radius, color);
        painter.circle_stroke(position, radius + 1.0, egui::Stroke::new(1.0, outline));
    }
}

struct SampledBezier {
    points: [egui::Pos2; 33],
    cumulative_lengths: [f32; 33],
    length: f32,
}

impl SampledBezier {
    fn new(control_points: [egui::Pos2; 4]) -> Self {
        let mut points = [control_points[0]; 33];
        let mut cumulative_lengths = [0.0; 33];

        for index in 1..=32 {
            let t = index as f32 / 32.0;
            points[index] = cubic_bezier_point(control_points, t);
            cumulative_lengths[index] =
                cumulative_lengths[index - 1] + points[index - 1].distance(points[index]);
        }

        Self {
            points,
            cumulative_lengths,
            length: cumulative_lengths[32],
        }
    }

    fn point_at_distance_fraction(&self, distance_fraction: f32) -> egui::Pos2 {
        let target = self.length * distance_fraction.clamp(0.0, 1.0);
        for index in 1..=32 {
            if self.cumulative_lengths[index] >= target {
                let segment_start = self.cumulative_lengths[index - 1];
                let segment_length = self.cumulative_lengths[index] - segment_start;
                let local_t = if segment_length <= f32::EPSILON {
                    0.0
                } else {
                    (target - segment_start) / segment_length
                };
                return self.points[index - 1].lerp(self.points[index], local_t);
            }
        }

        self.points[32]
    }
}

fn cubic_bezier_point(points: [egui::Pos2; 4], t: f32) -> egui::Pos2 {
    let t = t.clamp(0.0, 1.0);
    let inverse = 1.0 - t;
    let [p0, p1, p2, p3] = points;
    let point = p0.to_vec2() * inverse.powi(3)
        + p1.to_vec2() * 3.0 * inverse.powi(2) * t
        + p2.to_vec2() * 3.0 * inverse * t.powi(2)
        + p3.to_vec2() * t.powi(3);
    point.to_pos2()
}
