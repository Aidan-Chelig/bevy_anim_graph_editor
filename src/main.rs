use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, egui};
use egui_graph_edit::NodeResponse;

mod animation_graph;
mod preview;

use animation_graph::{AnimGraphEditor, AnimGraphResponse};
use preview::PreviewState;

const DEFAULT_GRAPH_PATH: &str = "assets/graphs/default.animgraph_editor.ron";

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Bevy Animation Graph Editor".to_string(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .init_resource::<AnimGraphEditor>()
        .add_systems(Startup, preview::setup_preview_scene)
        .add_systems(
            Update,
            (
                preview::build_preview_animation_graph,
                preview::update_preview_diagnostics,
                preview::attach_preview_animation_graph,
                preview::apply_editor_graph_to_preview,
                preview::sync_editor_graph_to_preview,
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
) -> Result {
    let ctx = contexts.ctx_mut()?;
    ctx.set_visuals(egui::Visuals::dark());
    let editor = editor.as_mut();

    egui::TopBottomPanel::top("main_menu").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.heading("Animation Graph");
            ui.separator();
            ui.label(&editor.last_event);
            ui.separator();
            if let Some(preview) = preview.as_deref_mut() {
                ui.label(&preview.status);
            }
            ui.separator();
            if ui.button("Apply Graph").clicked() {
                if let Some(preview) = preview.as_deref_mut() {
                    preview.apply_requested = true;
                    editor.last_event = "Applying graph to preview".to_string();
                } else {
                    editor.last_event = "Preview not initialized".to_string();
                }
            }
            ui.separator();
            if ui.button("Save").clicked() {
                match editor.save_to_path(DEFAULT_GRAPH_PATH) {
                    Ok(()) => editor.last_event = format!("Saved {DEFAULT_GRAPH_PATH}"),
                    Err(error) => editor.last_event = format!("Save failed: {error}"),
                }
            }
            if ui.button("Load").clicked() {
                match editor.load_from_path(DEFAULT_GRAPH_PATH) {
                    Ok(()) => {
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = format!("Loaded {DEFAULT_GRAPH_PATH}");
                    }
                    Err(error) => editor.last_event = format!("Load failed: {error}"),
                }
            }
            ui.separator();
            if ui.button("Reset View").clicked() {
                editor.graph.reset_zoom(ui);
                editor.graph.pan_zoom.pan = egui::Vec2::ZERO;
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

                if let Some(clip_node) = editor.selected_clip_node() {
                    ui.separator();
                    ui.heading("Clip");
                    if let Some(label) = editor.clip_node_label(clip_node) {
                        ui.label(format!("Selected: {label}"));
                    }
                    let mut clip_to_set = None;
                    if let Some(preview_state) = preview.as_deref() {
                        if let Some(gltf) = preview::loaded_gltf(preview_state, &gltfs) {
                            let clips = preview::clip_names(gltf);
                            if let Some(active) = clips.get(preview_state.active_animation)
                                && ui.button("Use active preview clip").clicked()
                            {
                                clip_to_set = Some(active.clone());
                            }
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .max_height(180.0)
                                .show(ui, |ui| {
                                    for clip in clips {
                                        if ui.button(&clip).clicked() {
                                            clip_to_set = Some(clip);
                                        }
                                    }
                                });
                        }
                    }
                    if let Some(clip) = clip_to_set {
                        editor.set_clip_node_label(clip_node, clip.clone());
                        if let Some(preview) = preview.as_deref_mut() {
                            preview.last_applied_signature = None;
                        }
                        editor.last_event = format!("Clip set to {clip}");
                    }
                }
            } else {
                ui.separator();
                ui.label("No node selected");
            }
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ctx, |ui| {
            let response = editor.graph.draw_graph_editor(
                ui,
                editor.templates,
                &mut editor.ui_state,
                Vec::new(),
            );

            for event in response.node_responses {
                match event {
                    NodeResponse::CreatedNode(_) => {
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
        });

    Ok(())
}
