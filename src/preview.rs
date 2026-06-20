use std::{fs, io, path::Path, time::Duration};

use bevy::{
    gltf::{Gltf, GltfAssetLabel},
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
};
use bevy_anim_graph_editor::{
    animation_graph::AnimGraphEditor,
    runtime::{self, LiveClipNode, LiveNodeWeight, LiveOneShot, LiveTransition},
};

const IMPORT_DIR: &str = "assets/imports";

#[derive(Resource)]
pub struct PreviewState {
    pub asset_path: Option<String>,
    pub gltf: Option<Handle<Gltf>>,
    pub graph: Option<Handle<AnimationGraph>>,
    pub animations: Vec<AnimationNodeIndex>,
    pub animation_names: Vec<String>,
    pub native_node_names: Vec<(AnimationNodeIndex, String)>,
    pub active_animation: usize,
    pub live_clips: Vec<LiveClipNode>,
    pub live_node_weights: Vec<LiveNodeWeight>,
    pub live_transitions: Vec<LiveTransition>,
    pub live_one_shots: Vec<LiveOneShot>,
    pub handled_completions: Vec<egui_graph_edit::NodeId>,
    pub scene_count: usize,
    pub player_count: usize,
    pub playback_active: bool,
    pub apply_requested: bool,
    pub auto_apply: bool,
    pub last_applied_signature: Option<String>,
    pub status: String,
    pub reload_scene: bool,
}

impl PreviewState {
    fn empty() -> Self {
        Self {
            asset_path: None,
            gltf: None,
            graph: None,
            animations: Vec::new(),
            animation_names: Vec::new(),
            native_node_names: Vec::new(),
            active_animation: 0,
            live_clips: Vec::new(),
            live_node_weights: Vec::new(),
            live_transitions: Vec::new(),
            live_one_shots: Vec::new(),
            handled_completions: Vec::new(),
            scene_count: 0,
            player_count: 0,
            playback_active: false,
            apply_requested: false,
            auto_apply: true,
            last_applied_signature: None,
            status: "No GLB loaded".to_string(),
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
        self.asset_path = Some(asset_path.clone());
        self.gltf = Some(asset_server.load(asset_path.clone()));
        self.graph = None;
        self.animations.clear();
        self.animation_names.clear();
        self.native_node_names.clear();
        self.live_clips.clear();
        self.live_node_weights.clear();
        self.live_transitions.clear();
        self.live_one_shots.clear();
        self.handled_completions.clear();
        self.active_animation = 0;
        self.scene_count = 0;
        self.player_count = 0;
        self.last_applied_signature = None;
        self.status = format!("Loading {asset_path}");
        self.reload_scene = true;
    }
}

#[derive(Component)]
pub struct PreviewCamera;

#[derive(Component)]
pub struct PreviewCameraRig {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for PreviewCameraRig {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 1.0, 0.0),
            yaw: 0.0,
            pitch: 0.15,
            distance: 5.5,
        }
    }
}

#[derive(Component)]
pub struct PreviewSceneRoot;

pub fn setup_preview_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(PreviewState::empty());

    let camera_rig = PreviewCameraRig::default();
    commands.spawn((
        Camera3d::default(),
        preview_camera_transform(&camera_rig),
        PreviewCamera,
        camera_rig,
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
}

pub fn control_preview_camera(
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    mouse_scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<(&mut PreviewCameraRig, &mut Transform), With<PreviewCamera>>,
) {
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if !shift {
        return;
    }

    let Ok((mut rig, mut transform)) = cameras.single_mut() else {
        return;
    };

    let motion = mouse_motion.delta;
    if mouse_buttons.pressed(MouseButton::Left) && motion.length_squared() > 0.0 {
        rig.yaw -= motion.x * 0.008;
        rig.pitch = (rig.pitch - motion.y * 0.008).clamp(-1.35, 1.35);
    }

    if mouse_buttons.pressed(MouseButton::Right) && motion.length_squared() > 0.0 {
        let right = transform.rotation * Vec3::X;
        let up = transform.rotation * Vec3::Y;
        let pan_scale = rig.distance * 0.0015;
        rig.target += (-right * motion.x + up * motion.y) * pan_scale;
    }

    if mouse_scroll.delta.y.abs() > f32::EPSILON {
        let scroll_lines = match mouse_scroll.unit {
            MouseScrollUnit::Line => mouse_scroll.delta.y,
            MouseScrollUnit::Pixel => mouse_scroll.delta.y / 32.0,
        };
        rig.distance = (rig.distance * (1.0 - scroll_lines * 0.08)).clamp(0.35, 80.0);
    }

    *transform = preview_camera_transform(&rig);
}

fn preview_camera_transform(rig: &PreviewCameraRig) -> Transform {
    let rotation =
        Quat::from_axis_angle(Vec3::Y, rig.yaw) * Quat::from_axis_angle(Vec3::X, rig.pitch);
    let offset = rotation * Vec3::new(0.0, 0.0, rig.distance);
    Transform::from_translation(rig.target + offset).looking_at(rig.target, Vec3::Y)
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

    let Some(asset_path) = state.asset_path.clone() else {
        return;
    };
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

    let Some(gltf_handle) = state.gltf.as_ref() else {
        return;
    };
    let Some(gltf) = gltfs.get(gltf_handle) else {
        return;
    };

    state.scene_count = gltf.scenes.len();

    if gltf.animations.is_empty() {
        state.status = "Loaded GLB with no animations".to_string();
        state.graph = Some(graphs.add(AnimationGraph::new()));
        return;
    }

    let (graph, animations) = AnimationGraph::from_clips(gltf.animations.clone());
    state.graph = Some(graphs.add(graph));
    state.animations = animations;
    state.animation_names = animation_names(gltf);
    state.native_node_names = state
        .animations
        .iter()
        .copied()
        .zip(state.animation_names.iter().cloned())
        .collect();
    state.live_clips.clear();
    state.live_node_weights.clear();
    state.live_transitions.clear();
    state.live_one_shots.clear();
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
    state.playback_active = players.iter().any(|player| {
        player
            .playing_animations()
            .any(|(_, animation)| !animation.is_paused() && !animation.is_finished())
    });
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

    let manual_apply = state.apply_requested;
    let signature = if manual_apply || (state.auto_apply && editor.is_changed()) {
        runtime::preview_tree_signature(&editor)
    } else {
        None
    };
    let should_auto_apply =
        state.auto_apply && signature.is_some() && signature != state.last_applied_signature;

    if !manual_apply && !should_auto_apply {
        return;
    }
    state.apply_requested = false;

    let Some(gltf_handle) = state.gltf.as_ref() else {
        state.status = "Cannot apply graph until a GLB is loaded".to_string();
        return;
    };
    let Some(gltf) = gltfs.get(gltf_handle) else {
        state.status = "Cannot apply graph until GLB is loaded".to_string();
        return;
    };

    match runtime::compile_editor_graph(&editor, gltf) {
        Ok(compiled) => {
            let graph_handle = graphs.add(compiled.graph);
            state.graph = Some(graph_handle.clone());
            state.animations = compiled.playable_nodes;
            state.animation_names = compiled.playable_names;
            state.native_node_names = compiled.native_node_names;
            state.live_clips = compiled.live_clips;
            state.live_node_weights = compiled.live_node_weights;
            state.live_transitions = compiled.live_transitions;
            state.live_one_shots = compiled.live_one_shots;
            state.handled_completions.clear();
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
    time: Res<Time>,
    mut editor: ResMut<AnimGraphEditor>,
    state: Option<ResMut<PreviewState>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    animation_clips: Res<Assets<AnimationClip>>,
    mut players: Query<&mut AnimationPlayer>,
) {
    let Some(mut state) = state else {
        return;
    };

    if state.live_clips.is_empty()
        && state.live_node_weights.is_empty()
        && state.live_transitions.is_empty()
        && state.live_one_shots.is_empty()
    {
        return;
    }

    if let Some(graph_handle) = state.graph.as_ref()
        && let Some(graph) = graphs.get_mut(graph_handle)
    {
        let live_node_weights = state.live_node_weights.clone();
        let mut live_transitions = std::mem::take(&mut state.live_transitions);
        let mut live_one_shots = std::mem::take(&mut state.live_one_shots);
        runtime::tick_animation_graph(
            &editor,
            graph,
            &live_node_weights,
            &mut live_transitions,
            &mut live_one_shots,
            time.delta_secs(),
        );
        state.live_transitions = live_transitions;
        state.live_one_shots = live_one_shots;
    }

    let graph = state
        .graph
        .as_ref()
        .and_then(|graph_handle| graphs.get(graph_handle));

    for mut player in &mut players {
        let live_clips = state.live_clips.clone();
        let events = runtime::apply_completion_actions(
            &mut editor,
            &live_clips,
            &mut state.handled_completions,
            &player,
            graph,
        );
        if let Some(event) = events.last() {
            state.status = event.clone();
        }

        let mut clip_durations = Vec::new();
        for live_clip in &state.live_clips {
            if !player.is_playing_animation(live_clip.animation) {
                player.play(live_clip.animation).repeat();
            }

            let Some(active_animation) = player.animation_mut(live_clip.animation) else {
                continue;
            };

            if state.live_one_shots.iter().any(|one_shot| {
                one_shot.restart_requested && live_clip.playback_node == one_shot.editor_node
            }) {
                active_animation.replay();
                active_animation.resume();
            }

            let clip_duration = graph
                .and_then(|graph| graph.get(live_clip.animation))
                .and_then(|node| match &node.node_type {
                    AnimationNodeType::Clip(handle) => animation_clips.get(handle),
                    _ => None,
                })
                .map(AnimationClip::duration)
                .unwrap_or(0.0);
            clip_durations.push((live_clip.animation, clip_duration));
            runtime::apply_clip_playback(&editor, *live_clip, active_animation, clip_duration);
        }

        let live_clips = state.live_clips.clone();
        runtime::update_one_shot_completion_targets(
            &live_clips,
            &mut state.live_one_shots,
            &player,
            &clip_durations,
        );
        for one_shot in &mut state.live_one_shots {
            one_shot.restart_requested = false;
        }
    }

    if editor.consume_trigger_parameters() {
        state.handled_completions.clear();
    }
}

pub use runtime::validate_editor_graph;

pub fn clip_names(gltf: &Gltf) -> Vec<String> {
    animation_names(gltf)
}

pub fn loaded_gltf<'a>(state: &PreviewState, gltfs: &'a Assets<Gltf>) -> Option<&'a Gltf> {
    gltfs.get(state.gltf.as_ref()?)
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
        let animations: Vec<_> = player
            .playing_animations()
            .map(|(animation, _)| *animation)
            .collect();
        if animations.is_empty() {
            continue;
        };

        let should_resume = animations.iter().all(|animation| {
            player
                .animation(*animation)
                .is_none_or(|animation| animation.is_paused())
        });

        for animation in animations {
            let Some(active_animation) = player.animation_mut(animation) else {
                continue;
            };
            if should_resume {
                active_animation.resume();
            } else {
                active_animation.pause();
            }
        }

        if should_resume {
            // Restart one-shot animations that had already completed before being resumed.
            for animation in player
                .playing_animations()
                .filter_map(|(animation, active)| active.is_finished().then_some(*animation))
                .collect::<Vec<_>>()
            {
                if let Some(active_animation) = player.animation_mut(animation) {
                    active_animation.replay();
                }
            }
        }
    }
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
