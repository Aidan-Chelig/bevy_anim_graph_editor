use bevy::{gltf::Gltf, prelude::*};

use crate::animation_graph::{AnimGraphEditor, AnimNodeTemplate, AnimValue};

pub struct AnimGraphRuntimePlugin;

impl Plugin for AnimGraphRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (compile_runtime_graphs, sync_runtime_graphs).chain(),
        );
    }
}

#[derive(Component)]
pub struct AnimGraphRuntime {
    pub editor: AnimGraphEditor,
    pub gltf: Handle<Gltf>,
    pub graph: Option<Handle<AnimationGraph>>,
    pub playable_nodes: Vec<AnimationNodeIndex>,
    pub playable_names: Vec<String>,
    pub live_clips: Vec<LiveClipNode>,
    pub live_node_weights: Vec<LiveNodeWeight>,
    pub status: String,
}

impl AnimGraphRuntime {
    pub fn new(editor: AnimGraphEditor, gltf: Handle<Gltf>) -> Self {
        Self {
            editor,
            gltf,
            graph: None,
            playable_nodes: Vec::new(),
            playable_names: Vec::new(),
            live_clips: Vec::new(),
            live_node_weights: Vec::new(),
            status: "Waiting for GLB".to_string(),
        }
    }

    pub fn request_rebuild(&mut self) {
        self.graph = None;
        self.playable_nodes.clear();
        self.playable_names.clear();
        self.live_clips.clear();
        self.live_node_weights.clear();
        self.status = "Rebuild requested".to_string();
    }
}

#[derive(Clone, Copy)]
pub struct LiveClipNode {
    pub editor_node: egui_graph_edit::NodeId,
    pub animation: AnimationNodeIndex,
}

#[derive(Clone, Copy)]
pub struct LiveNodeWeight {
    pub animation: AnimationNodeIndex,
    pub driver: WeightDriver,
}

#[derive(Clone, Copy)]
pub enum WeightDriver {
    BlendA(egui_graph_edit::NodeId),
    BlendB(egui_graph_edit::NodeId),
    TransitionFrom(egui_graph_edit::NodeId),
    TransitionTo(egui_graph_edit::NodeId),
}

impl WeightDriver {
    pub fn resolve(self, editor: &AnimGraphEditor) -> f32 {
        match self {
            Self::BlendA(node) => 1.0 - blend_weight(editor, node),
            Self::BlendB(node) => blend_weight(editor, node),
            Self::TransitionFrom(node) => {
                if resolve_bool_input(editor, node, "Condition").unwrap_or(false) {
                    0.0
                } else {
                    1.0
                }
            }
            Self::TransitionTo(node) => {
                if resolve_bool_input(editor, node, "Condition").unwrap_or(false) {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

pub struct CompiledEditorGraph {
    pub graph: AnimationGraph,
    pub playable_nodes: Vec<AnimationNodeIndex>,
    pub playable_names: Vec<String>,
    pub live_clips: Vec<LiveClipNode>,
    pub live_node_weights: Vec<LiveNodeWeight>,
    pub summary: String,
}

pub struct GraphValidation {
    pub can_apply: bool,
    pub message: String,
}

pub fn compile_runtime_graphs(
    mut commands: Commands,
    mut runtimes: Query<(Entity, &mut AnimGraphRuntime, &mut AnimationPlayer)>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for (entity, mut runtime, mut player) in &mut runtimes {
        if runtime.graph.is_some() {
            continue;
        }

        let Some(gltf) = gltfs.get(&runtime.gltf) else {
            runtime.status = "Waiting for GLB".to_string();
            continue;
        };

        match compile_editor_graph(&runtime.editor, gltf) {
            Ok(compiled) => {
                let graph_handle = graphs.add(compiled.graph);
                runtime.graph = Some(graph_handle.clone());
                runtime.playable_nodes = compiled.playable_nodes;
                runtime.playable_names = compiled.playable_names;
                runtime.live_clips = compiled.live_clips;
                runtime.live_node_weights = compiled.live_node_weights;
                runtime.status = format!("Applied graph: {}", compiled.summary);

                commands
                    .entity(entity)
                    .insert(AnimationGraphHandle(graph_handle));

                for animation in runtime.playable_nodes.iter().copied() {
                    if !player.is_playing_animation(animation) {
                        player.play(animation).repeat();
                    }
                }
            }
            Err(error) => {
                runtime.status = format!("Graph apply failed: {error}");
            }
        }
    }
}

pub fn sync_runtime_graphs(
    runtimes: Query<(&AnimGraphRuntime, &AnimationGraphHandle)>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut players: Query<(&AnimGraphRuntime, &mut AnimationPlayer)>,
) {
    for (runtime, graph_handle) in &runtimes {
        if let Some(graph) = graphs.get_mut(graph_handle) {
            sync_animation_graph_weights(&runtime.editor, graph, &runtime.live_node_weights);
        }
    }

    for (runtime, mut player) in &mut players {
        for live_clip in &runtime.live_clips {
            if !player.is_playing_animation(live_clip.animation) {
                player.play(live_clip.animation).repeat();
            }

            let Some(active_animation) = player.animation_mut(live_clip.animation) else {
                continue;
            };

            active_animation.set_weight(1.0);
            active_animation.set_speed(clip_speed(&runtime.editor, live_clip.editor_node));
        }
    }
}

pub fn validate_editor_graph(editor: &AnimGraphEditor, gltf: &Gltf) -> GraphValidation {
    match compile_editor_graph(editor, gltf) {
        Ok(compiled) => GraphValidation {
            can_apply: true,
            message: format!("Ready: {}", compiled.summary),
        },
        Err(error) => GraphValidation {
            can_apply: false,
            message: error,
        },
    }
}

pub fn compile_editor_graph(
    editor: &AnimGraphEditor,
    gltf: &Gltf,
) -> Result<CompiledEditorGraph, String> {
    let output = preview_output_node(editor).ok_or_else(|| "no Output node exists".to_string())?;

    let input = editor.graph.graph.nodes[output]
        .get_input("Pose")
        .map_err(|_| "Output node has no Pose input".to_string())?;
    let source = editor
        .graph
        .graph
        .connection(input)
        .ok_or_else(|| "Output Pose input is not connected".to_string())?;

    let mut graph = AnimationGraph::new();
    let mut playable_nodes = Vec::new();
    let mut playable_names = Vec::new();
    let mut live_clips = Vec::new();
    let mut live_node_weights = Vec::new();
    let root = graph.root;
    compile_source_node(
        editor,
        gltf,
        source,
        &mut graph,
        root,
        &mut playable_nodes,
        &mut playable_names,
        &mut live_clips,
        &mut live_node_weights,
        None,
    )?;

    let summary = if playable_names.is_empty() {
        "no playable clip nodes".to_string()
    } else {
        playable_names.join(" + ")
    };

    Ok(CompiledEditorGraph {
        graph,
        playable_nodes,
        playable_names,
        live_clips,
        live_node_weights,
        summary,
    })
}

pub fn sync_animation_graph_weights(
    editor: &AnimGraphEditor,
    graph: &mut AnimationGraph,
    live_node_weights: &[LiveNodeWeight],
) {
    for live_weight in live_node_weights {
        if let Some(node) = graph.get_mut(live_weight.animation) {
            node.weight = live_weight.driver.resolve(editor).clamp(0.0, 1.0);
        }
    }
}

pub fn clip_speed(editor: &AnimGraphEditor, clip_node: egui_graph_edit::NodeId) -> f32 {
    node_input_float(editor, clip_node, "Speed")
        .unwrap_or(1.0)
        .max(0.0)
}

pub fn preview_tree_signature(editor: &AnimGraphEditor) -> Option<String> {
    let output = preview_output_node(editor)?;
    let input = editor.graph.graph.nodes[output].get_input("Pose").ok()?;
    let source = editor.graph.graph.connection(input)?;
    let mut signature = format!("output:{:?};", output);
    append_output_signature(editor, source, &mut signature);
    Some(signature)
}

fn compile_source_node(
    editor: &AnimGraphEditor,
    gltf: &Gltf,
    output: egui_graph_edit::OutputId,
    graph: &mut AnimationGraph,
    parent: AnimationNodeIndex,
    playable_nodes: &mut Vec<AnimationNodeIndex>,
    playable_names: &mut Vec<String>,
    live_clips: &mut Vec<LiveClipNode>,
    live_node_weights: &mut Vec<LiveNodeWeight>,
    weight_driver: Option<WeightDriver>,
) -> Result<AnimationNodeIndex, String> {
    let node_id = editor.graph.graph.get_output(output).node;
    let node = &editor.graph.graph.nodes[node_id];
    let initial_weight = weight_driver
        .map(|driver| driver.resolve(editor))
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);

    match node.user_data.template {
        AnimNodeTemplate::Clip => {
            let clip_label = node_input_text(editor, node_id, "Clip").unwrap_or_default();
            let (clip, clip_name) = resolve_clip(gltf, &clip_label)?;
            let node = graph.add_clip(clip, initial_weight, parent);
            if let Some(driver) = weight_driver {
                live_node_weights.push(LiveNodeWeight {
                    animation: node,
                    driver,
                });
            }
            playable_nodes.push(node);
            playable_names.push(clip_name);
            live_clips.push(LiveClipNode {
                editor_node: node_id,
                animation: node,
            });
            Ok(node)
        }
        AnimNodeTemplate::Blend => {
            let blend = graph.add_blend(initial_weight, parent);
            if let Some(driver) = weight_driver {
                live_node_weights.push(LiveNodeWeight {
                    animation: blend,
                    driver,
                });
            }
            compile_connected_input(
                editor,
                gltf,
                node_id,
                "A",
                graph,
                blend,
                playable_nodes,
                playable_names,
                live_clips,
                live_node_weights,
                Some(WeightDriver::BlendA(node_id)),
            )?;
            compile_connected_input(
                editor,
                gltf,
                node_id,
                "B",
                graph,
                blend,
                playable_nodes,
                playable_names,
                live_clips,
                live_node_weights,
                Some(WeightDriver::BlendB(node_id)),
            )?;
            Ok(blend)
        }
        AnimNodeTemplate::State => compile_connected_input(
            editor,
            gltf,
            node_id,
            "Pose",
            graph,
            parent,
            playable_nodes,
            playable_names,
            live_clips,
            live_node_weights,
            weight_driver,
        ),
        AnimNodeTemplate::Transition => {
            let transition = graph.add_blend(initial_weight, parent);
            if let Some(driver) = weight_driver {
                live_node_weights.push(LiveNodeWeight {
                    animation: transition,
                    driver,
                });
            }
            compile_connected_input(
                editor,
                gltf,
                node_id,
                "From",
                graph,
                transition,
                playable_nodes,
                playable_names,
                live_clips,
                live_node_weights,
                Some(WeightDriver::TransitionFrom(node_id)),
            )?;
            compile_connected_input(
                editor,
                gltf,
                node_id,
                "To",
                graph,
                transition,
                playable_nodes,
                playable_names,
                live_clips,
                live_node_weights,
                Some(WeightDriver::TransitionTo(node_id)),
            )?;
            Ok(transition)
        }
        AnimNodeTemplate::FloatParameter
        | AnimNodeTemplate::BoolParameter
        | AnimNodeTemplate::Remap
        | AnimNodeTemplate::Output => Err(format!("{} does not produce a pose", node.label)),
    }
}

fn compile_connected_input(
    editor: &AnimGraphEditor,
    gltf: &Gltf,
    node: egui_graph_edit::NodeId,
    input_name: &str,
    graph: &mut AnimationGraph,
    parent: AnimationNodeIndex,
    playable_nodes: &mut Vec<AnimationNodeIndex>,
    playable_names: &mut Vec<String>,
    live_clips: &mut Vec<LiveClipNode>,
    live_node_weights: &mut Vec<LiveNodeWeight>,
    weight_driver: Option<WeightDriver>,
) -> Result<AnimationNodeIndex, String> {
    let input = editor.graph.graph.nodes[node]
        .get_input(input_name)
        .map_err(|_| format!("missing input {input_name}"))?;
    let output = editor
        .graph
        .graph
        .connection(input)
        .ok_or_else(|| format!("input {input_name} is not connected"))?;
    compile_source_node(
        editor,
        gltf,
        output,
        graph,
        parent,
        playable_nodes,
        playable_names,
        live_clips,
        live_node_weights,
        weight_driver,
    )
}

fn preview_output_node(editor: &AnimGraphEditor) -> Option<egui_graph_edit::NodeId> {
    editor.preview_output.or_else(|| {
        editor.graph.graph.nodes.iter().find_map(|(id, node)| {
            matches!(node.user_data.template, AnimNodeTemplate::Output).then_some(id)
        })
    })
}

fn append_output_signature(
    editor: &AnimGraphEditor,
    output: egui_graph_edit::OutputId,
    signature: &mut String,
) {
    let node_id = editor.graph.graph.get_output(output).node;
    let node = &editor.graph.graph.nodes[node_id];
    signature.push_str(&format!("{:?}:{:?};", node_id, node.user_data.template));

    match node.user_data.template {
        AnimNodeTemplate::Clip => {
            signature.push_str("clip:");
            signature.push_str(
                node_input_text(editor, node_id, "Clip")
                    .as_deref()
                    .unwrap_or(""),
            );
            signature.push(';');
        }
        AnimNodeTemplate::Blend => {
            append_connected_signature(editor, node_id, "A", signature);
            append_connected_signature(editor, node_id, "B", signature);
        }
        AnimNodeTemplate::State => {
            append_connected_signature(editor, node_id, "Pose", signature);
        }
        AnimNodeTemplate::Transition => {
            append_connected_signature(editor, node_id, "From", signature);
            append_connected_signature(editor, node_id, "To", signature);
        }
        AnimNodeTemplate::FloatParameter
        | AnimNodeTemplate::BoolParameter
        | AnimNodeTemplate::Remap
        | AnimNodeTemplate::Output => {}
    }
}

fn append_connected_signature(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
    signature: &mut String,
) {
    signature.push_str(input_name);
    signature.push(':');
    let Ok(input) = editor.graph.graph.nodes[node].get_input(input_name) else {
        signature.push_str("missing;");
        return;
    };
    let Some(output) = editor.graph.graph.connection(input) else {
        signature.push_str("disconnected;");
        return;
    };
    append_output_signature(editor, output, signature);
}

fn node_input_text(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
) -> Option<String> {
    let input = editor.graph.graph.nodes[node].get_input(input_name).ok()?;
    match editor.graph.graph.get_input(input).value() {
        AnimValue::Text(value) => Some(value.clone()),
        _ => None,
    }
}

fn node_input_float(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
) -> Option<f32> {
    let input = editor.graph.graph.nodes[node].get_input(input_name).ok()?;
    match editor.graph.graph.get_input(input).value() {
        AnimValue::Float(value) => Some(*value),
        _ => None,
    }
}

fn node_input_bool(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
) -> Option<bool> {
    let input = editor.graph.graph.nodes[node].get_input(input_name).ok()?;
    match editor.graph.graph.get_input(input).value() {
        AnimValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn blend_weight(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> f32 {
    resolve_float_input(editor, node, "Weight")
        .unwrap_or(0.5)
        .clamp(0.0, 1.0)
}

fn resolve_float_input(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
) -> Option<f32> {
    let input = editor.graph.graph.nodes[node].get_input(input_name).ok()?;
    if let Some(output) = editor.graph.graph.connection(input) {
        return resolve_float_output(editor, output);
    }

    node_input_float(editor, node, input_name)
}

fn resolve_bool_input(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
) -> Option<bool> {
    let input = editor.graph.graph.nodes[node].get_input(input_name).ok()?;
    if let Some(output) = editor.graph.graph.connection(input) {
        return resolve_bool_output(editor, output);
    }

    node_input_bool(editor, node, input_name)
}

fn resolve_float_output(
    editor: &AnimGraphEditor,
    output: egui_graph_edit::OutputId,
) -> Option<f32> {
    let node = editor.graph.graph.get_output(output).node;
    match editor.graph.graph.nodes[node].user_data.template {
        AnimNodeTemplate::FloatParameter => node_input_float(editor, node, "Value"),
        AnimNodeTemplate::Remap => {
            let value = resolve_float_input(editor, node, "Value")?;
            let in_min = node_input_float(editor, node, "In Min").unwrap_or(0.0);
            let in_max = node_input_float(editor, node, "In Max").unwrap_or(1.0);
            let range = in_max - in_min;

            if range.abs() <= f32::EPSILON {
                Some(if value >= in_max { 1.0 } else { 0.0 })
            } else {
                Some(((value - in_min) / range).clamp(0.0, 1.0))
            }
        }
        _ => None,
    }
}

fn resolve_bool_output(
    editor: &AnimGraphEditor,
    output: egui_graph_edit::OutputId,
) -> Option<bool> {
    let node = editor.graph.graph.get_output(output).node;
    match editor.graph.graph.nodes[node].user_data.template {
        AnimNodeTemplate::BoolParameter => node_input_bool(editor, node, "Value"),
        _ => None,
    }
}

fn resolve_clip(gltf: &Gltf, label: &str) -> Result<(Handle<AnimationClip>, String), String> {
    if gltf.animations.is_empty() {
        return Err("character.glb has no animations".to_string());
    }

    let requested = label
        .rsplit_once('#')
        .map(|(_, name)| name)
        .unwrap_or(label)
        .trim();

    if requested.is_empty() {
        return Ok((gltf.animations[0].clone(), "Animation 0".to_string()));
    }

    if let Ok(index) = requested
        .strip_prefix("Animation ")
        .unwrap_or(requested)
        .parse::<usize>()
        && let Some(clip) = gltf.animations.get(index)
    {
        return Ok((clip.clone(), format!("Animation {index}")));
    }

    if let Some((name, clip)) = gltf
        .named_animations
        .iter()
        .find(|(name, _)| name.as_ref() == requested)
    {
        return Ok((clip.clone(), name.to_string()));
    }

    Ok((
        gltf.animations[0].clone(),
        format!("{requested} -> Animation 0"),
    ))
}
