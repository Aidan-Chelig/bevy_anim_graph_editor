use std::{path::PathBuf, time::Duration};

use bevy::audio::AudioPlugin;
use bevy::prelude::*;
use bevy::winit::WinitSettings;
use bevy_anim_graph_editor::{
    animation_graph::{AnimGraphEditor, AnimGraphResponse, EdgeVisualization},
    runtime,
};
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use egui_graph_edit::NodeResponse;

mod app_io;
mod edge_visualization;
mod preview;

use app_io::{
    apply_graph, export_bevy_animation_graph, export_runtime_graph, import_gltf, load_project,
    save_project, save_project_as,
};
use edge_visualization::draw_connection_visualization;
use preview::{PreviewPlugin, PreviewState};

#[derive(Resource, Default)]
pub(crate) struct StartupInput {
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
        .add_plugins(PreviewPlugin)
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
    editor.sanitize_after_graph_change();

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

            if let Some(node) = editor
                .graph
                .selected_nodes
                .first()
                .and_then(|node_id| editor.graph.graph.nodes.get(*node_id))
            {
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
                        .map(|one_shot| ((one_shot.editor_node, one_shot.lane), one_shot.progress))
                        .collect()
                })
                .unwrap_or_default();
            editor.ui_state.one_shot_action_clip_labels = editor.one_shot_action_clip_labels();
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
                        editor.sanitize_after_graph_change();
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = format!("Deleted {}", node.label);
                    }
                    NodeResponse::User(AnimGraphResponse::SetOutput(node_id)) => {
                        editor.preview_output = Some(node_id);
                        editor.sanitize_after_graph_change();
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = "Preview output selected".to_string();
                    }
                    NodeResponse::User(AnimGraphResponse::AddOneShotLane(node_id)) => {
                        if editor.add_one_shot_lane(node_id) {
                            editor.sanitize_after_graph_change();
                            if let Some(preview) = preview.as_deref_mut() {
                                preview.last_applied_signature = None;
                            }
                            editor.last_event = "One shot lane added".to_string();
                        }
                    }
                    NodeResponse::User(AnimGraphResponse::RemoveOneShotLane(node_id)) => {
                        if editor.remove_one_shot_lane(node_id) {
                            editor.sanitize_after_graph_change();
                            if let Some(preview) = preview.as_deref_mut() {
                                preview.last_applied_signature = None;
                            }
                            editor.last_event = "One shot lane removed".to_string();
                        }
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
