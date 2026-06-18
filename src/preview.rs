use std::{fs, io, path::Path, time::Duration};

use bevy::{
    gltf::{Gltf, GltfAssetLabel},
    prelude::*,
};

use crate::animation_graph::{AnimGraphEditor, AnimNodeTemplate, AnimValue};

const PREVIEW_ASSET: &str = "character.glb";
const IMPORT_DIR: &str = "assets/imports";

#[derive(Resource)]
pub struct PreviewState {
    pub asset_path: String,
    pub gltf: Handle<Gltf>,
    pub graph: Option<Handle<AnimationGraph>>,
    pub animations: Vec<AnimationNodeIndex>,
    pub animation_names: Vec<String>,
    pub active_animation: usize,
    pub live_clips: Vec<LiveClipNode>,
    pub live_node_weights: Vec<LiveNodeWeight>,
    pub scene_count: usize,
    pub player_count: usize,
    pub apply_requested: bool,
    pub auto_apply: bool,
    pub last_applied_signature: Option<String>,
    pub status: String,
    pub reload_scene: bool,
}

impl PreviewState {
    fn new(asset_path: String, gltf: Handle<Gltf>) -> Self {
        Self {
            asset_path,
            gltf,
            graph: None,
            animations: Vec::new(),
            animation_names: Vec::new(),
            active_animation: 0,
            live_clips: Vec::new(),
            live_node_weights: Vec::new(),
            scene_count: 0,
            player_count: 0,
            apply_requested: false,
            auto_apply: true,
            last_applied_signature: None,
            status: "Loading character.glb".to_string(),
            reload_scene: false,
        }
    }

    pub fn import_gltf(
        &mut self,
        source: impl AsRef<Path>,
        asset_server: &AssetServer,
    ) -> Result<(), ImportGltfError> {
        let imported_asset = import_gltf_asset(source)?;
        self.load_asset_path(imported_asset, asset_server);
        Ok(())
    }

    pub fn load_asset_path(&mut self, asset_path: String, asset_server: &AssetServer) {
        self.asset_path = asset_path.clone();
        self.gltf = asset_server.load(asset_path.clone());
        self.graph = None;
        self.animations.clear();
        self.animation_names.clear();
        self.live_clips.clear();
        self.live_node_weights.clear();
        self.active_animation = 0;
        self.scene_count = 0;
        self.player_count = 0;
        self.last_applied_signature = None;
        self.status = format!("Loading {asset_path}");
        self.reload_scene = true;
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
    fn resolve(self, editor: &AnimGraphEditor) -> f32 {
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

#[derive(Component)]
struct PreviewCamera;

#[derive(Component)]
pub struct PreviewSceneRoot;

pub fn setup_preview_scene(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let gltf = asset_server.load(PREVIEW_ASSET);
    commands.insert_resource(PreviewState::new(PREVIEW_ASSET.to_string(), gltf));

    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 1.8, 5.5).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y),
        PreviewCamera,
    ));

    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(6.0, 6.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.18, 0.19, 0.19),
            perceptual_roughness: 0.8,
            ..default()
        })),
    ));

    commands.spawn((
        DirectionalLight {
            illuminance: 7500.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(2.5, 5.0, 3.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        PointLight {
            intensity: 600.0,
            radius: 5.0,
            ..default()
        },
        Transform::from_xyz(-2.0, 2.5, 2.0),
    ));

    commands.spawn((
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(PREVIEW_ASSET))),
        Transform::from_scale(Vec3::splat(1.0)),
        PreviewSceneRoot,
    ));
}

pub fn reload_preview_scene(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    state: Option<ResMut<PreviewState>>,
    scene_roots: Query<Entity, With<PreviewSceneRoot>>,
) {
    let Some(mut state) = state else {
        return;
    };

    if !state.reload_scene {
        return;
    }
    state.reload_scene = false;

    for entity in &scene_roots {
        commands.entity(entity).despawn();
    }

    let asset_path = state.asset_path.clone();
    commands.spawn((
        SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(asset_path))),
        Transform::from_scale(Vec3::splat(1.0)),
        PreviewSceneRoot,
    ));
}

pub fn build_preview_animation_graph(
    mut state: ResMut<PreviewState>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    if state.graph.is_some() {
        return;
    }

    let Some(gltf) = gltfs.get(&state.gltf) else {
        return;
    };

    state.scene_count = gltf.scenes.len();

    if gltf.animations.is_empty() {
        state.status = "Loaded character.glb with no animations".to_string();
        state.graph = Some(graphs.add(AnimationGraph::new()));
        return;
    }

    let (graph, animations) = AnimationGraph::from_clips(gltf.animations.clone());
    state.graph = Some(graphs.add(graph));
    state.animations = animations;
    state.animation_names = animation_names(gltf);
    state.live_clips.clear();
    state.live_node_weights.clear();
    state.active_animation = 0;
    state.status = format!("Loaded {} animation(s)", state.animations.len());
}

pub fn update_preview_diagnostics(
    state: Option<ResMut<PreviewState>>,
    players: Query<&AnimationPlayer>,
) {
    let Some(mut state) = state else {
        return;
    };

    state.player_count = players.iter().count();
}

pub fn attach_preview_animation_graph(
    mut commands: Commands,
    mut players: Query<(Entity, &mut AnimationPlayer), Added<AnimationPlayer>>,
    state: Option<Res<PreviewState>>,
) {
    let Some(state) = state else {
        return;
    };

    let Some(graph) = state.graph.clone() else {
        return;
    };

    for (entity, mut player) in &mut players {
        let mut transitions = AnimationTransitions::new();

        if let Some(animation) = state.animations.first().copied() {
            transitions
                .play(&mut player, animation, Duration::ZERO)
                .repeat();
        }

        commands
            .entity(entity)
            .insert(AnimationGraphHandle(graph.clone()))
            .insert(transitions);
    }
}

pub fn cycle_preview_animation(
    input: Res<ButtonInput<KeyCode>>,
    state: Option<ResMut<PreviewState>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions)>,
) {
    let Some(mut state) = state else {
        return;
    };

    if !state.live_clips.is_empty()
        || state.animations.is_empty()
        || !input.just_pressed(KeyCode::Enter)
    {
        return;
    }

    state.active_animation = (state.active_animation + 1) % state.animations.len();
    let animation = state.animations[state.active_animation];

    for (mut player, mut transitions) in &mut players {
        transitions
            .play(&mut player, animation, Duration::from_millis(180))
            .repeat();
    }

    let name = state
        .animation_names
        .get(state.active_animation)
        .cloned()
        .unwrap_or_else(|| format!("Animation {}", state.active_animation));
    state.status = format!("Playing {name}");
}

pub fn apply_editor_graph_to_preview(
    editor: Res<AnimGraphEditor>,
    state: Option<ResMut<PreviewState>>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationGraphHandle)>,
) {
    let Some(mut state) = state else {
        return;
    };

    let signature = preview_tree_signature(&editor);
    let should_auto_apply =
        state.auto_apply && signature.is_some() && signature != state.last_applied_signature;
    let manual_apply = state.apply_requested;

    if !manual_apply && !should_auto_apply {
        return;
    }
    state.apply_requested = false;

    let Some(gltf) = gltfs.get(&state.gltf) else {
        state.status = "Cannot apply graph until character.glb is loaded".to_string();
        return;
    };

    match compile_editor_graph(&editor, gltf) {
        Ok(compiled) => {
            let graph_handle = graphs.add(compiled.graph);
            state.graph = Some(graph_handle.clone());
            state.animations = compiled.playable_nodes;
            state.animation_names = compiled.playable_names;
            state.live_clips = compiled.live_clips;
            state.live_node_weights = compiled.live_node_weights;
            state.active_animation = 0;
            state.last_applied_signature = signature;
            state.status = format!("Applied editor graph: {}", compiled.summary);

            for (mut player, mut handle) in &mut players {
                *handle = AnimationGraphHandle(graph_handle.clone());
                for animation in state.animations.iter().copied() {
                    if !player.is_playing_animation(animation) {
                        player.play(animation).repeat();
                    }
                }
            }
        }
        Err(error) => {
            state.status = if manual_apply {
                format!("Graph apply failed: {error}")
            } else {
                format!("Auto apply blocked: {error}")
            };
        }
    }
}

pub fn sync_editor_graph_to_preview(
    editor: Res<AnimGraphEditor>,
    state: Option<Res<PreviewState>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let Some(state) = state else {
        return;
    };

    if state.live_clips.is_empty() && state.live_node_weights.is_empty() {
        return;
    }

    if let Some(graph_handle) = state.graph.as_ref()
        && let Some(graph) = graphs.get_mut(graph_handle)
    {
        for live_weight in &state.live_node_weights {
            if let Some(node) = graph.get_mut(live_weight.animation) {
                node.weight = live_weight.driver.resolve(&editor).clamp(0.0, 1.0);
            }
        }
    }

    for mut player in &mut players {
        for live_clip in &state.live_clips {
            if !player.is_playing_animation(live_clip.animation) {
                player.play(live_clip.animation).repeat();
            }

            let Some(active_animation) = player.animation_mut(live_clip.animation) else {
                continue;
            };

            let speed = node_input_float(&editor, live_clip.editor_node, "Speed").unwrap_or(1.0);
            active_animation.set_weight(1.0);
            active_animation.set_speed(speed.max(0.0));
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

pub fn preview_tree_signature(editor: &AnimGraphEditor) -> Option<String> {
    let output = preview_output_node(editor)?;
    let input = editor.graph.graph.nodes[output].get_input("Pose").ok()?;
    let source = editor.graph.graph.connection(input)?;
    let mut signature = format!("output:{:?};", output);
    append_output_signature(editor, source, &mut signature);
    Some(signature)
}

pub fn clip_names(gltf: &Gltf) -> Vec<String> {
    animation_names(gltf)
}

pub fn loaded_gltf<'a>(state: &PreviewState, gltfs: &'a Assets<Gltf>) -> Option<&'a Gltf> {
    gltfs.get(&state.gltf)
}

pub fn asset_path_exists(asset_path: &str) -> bool {
    Path::new("assets").join(asset_path).exists()
}

pub fn toggle_preview_playback(
    input: Res<ButtonInput<KeyCode>>,
    mut players: Query<&mut AnimationPlayer>,
) {
    if !input.just_pressed(KeyCode::Space) {
        return;
    }

    for mut player in &mut players {
        let Some((animation, _)) = player.playing_animations().next() else {
            continue;
        };
        let animation = *animation;
        let Some(active_animation) = player.animation_mut(animation) else {
            continue;
        };

        if active_animation.is_paused() {
            active_animation.resume();
        } else {
            active_animation.pause();
        }
    }
}

pub struct GraphValidation {
    pub can_apply: bool,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ImportGltfError {
    #[error("selected file has no filename")]
    MissingFileName,
    #[error("selected file must be a .glb or .gltf")]
    UnsupportedExtension,
    #[error(transparent)]
    Io(#[from] io::Error),
}

struct CompiledEditorGraph {
    graph: AnimationGraph,
    playable_nodes: Vec<AnimationNodeIndex>,
    playable_names: Vec<String>,
    live_clips: Vec<LiveClipNode>,
    live_node_weights: Vec<LiveNodeWeight>,
    summary: String,
}

fn compile_editor_graph(
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
    {
        if let Some(clip) = gltf.animations.get(index) {
            return Ok((clip.clone(), format!("Animation {index}")));
        }
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

fn animation_names(gltf: &Gltf) -> Vec<String> {
    gltf.animations
        .iter()
        .enumerate()
        .map(|(index, handle)| {
            gltf.named_animations
                .iter()
                .find_map(|(name, named_handle)| (named_handle == handle).then(|| name.to_string()))
                .unwrap_or_else(|| format!("Animation {index}"))
        })
        .collect()
}

fn import_gltf_asset(source: impl AsRef<Path>) -> Result<String, ImportGltfError> {
    let source = source.as_ref();
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "glb" | "gltf") {
        return Err(ImportGltfError::UnsupportedExtension);
    }

    let file_name = source.file_name().ok_or(ImportGltfError::MissingFileName)?;
    fs::create_dir_all(IMPORT_DIR)?;
    let destination = Path::new(IMPORT_DIR).join(file_name);

    let source_canonical = source.canonicalize()?;
    let destination_canonical = destination.canonicalize().ok();
    if destination_canonical.as_ref() != Some(&source_canonical) {
        fs::copy(source, &destination)?;
    }

    Ok(format!("imports/{}", file_name.to_string_lossy()))
}
