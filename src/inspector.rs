//! Inspector shared by the standalone editor and embedded hosts.

use bevy::{gltf::Gltf, prelude::*};
use egui::Ui;

use crate::{
    animation_graph::{AnimGraphEditor, EdgeVisualization},
    preview::{self, PreviewState},
    runtime,
};

pub fn draw_inspector(
    ui: &mut Ui,
    editor: &mut AnimGraphEditor,
    mut preview: Option<&mut PreviewState>,
    gltfs: &Assets<Gltf>,
    graphs: &Assets<AnimationGraph>,
) {
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
        if let Some(gltf) = preview::loaded_gltf(preview, gltfs) {
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
                ui.colored_label(egui::Color32::from_rgb(255, 140, 110), validation.message);
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
                        for line in runtime::native_tree_lines(graph, &preview.native_node_names) {
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
}
