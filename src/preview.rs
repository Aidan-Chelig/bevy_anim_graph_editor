use std::{fs, io, path::Path, time::Duration};

use bevy::{
    gltf::{Gltf, GltfAssetLabel},
    prelude::*,
};
use bevy_anim_graph_editor::{
    animation_graph::AnimGraphEditor,
    runtime::{self, LiveClipNode, LiveNodeWeight},
};

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

    let signature = runtime::preview_tree_signature(&editor);
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

    match runtime::compile_editor_graph(&editor, gltf) {
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
        runtime::sync_animation_graph_weights(&editor, graph, &state.live_node_weights);
    }

    for mut player in &mut players {
        for live_clip in &state.live_clips {
            if !player.is_playing_animation(live_clip.animation) {
                player.play(live_clip.animation).repeat();
            }

            let Some(active_animation) = player.animation_mut(live_clip.animation) else {
                continue;
            };

            let speed = runtime::clip_speed(&editor, live_clip.editor_node);
            active_animation.set_weight(1.0);
            active_animation.set_speed(speed);
        }
    }
}

pub use runtime::validate_editor_graph;

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
