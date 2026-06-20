use bevy::{
    animation::{ActiveAnimation, RepeatAnimation},
    asset::{AssetApp, AssetLoader, LoadContext, io::Reader},
    gltf::Gltf,
    prelude::*,
    reflect::TypePath,
};

use crate::animation_graph::{
    AnimGraphEditor, AnimNodeTemplate, AnimValue, CompletionAction, MIN_TRANSITION_DURATION,
    PlaybackMode, SavedAnimGraph,
};

pub struct AnimGraphRuntimePlugin;

impl Plugin for AnimGraphRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<AnimGraphProjectAsset>()
            .register_asset_loader(AnimGraphProjectLoader)
            .add_systems(
                Update,
                (
                    hydrate_project_runtimes,
                    compile_runtime_graphs,
                    sync_runtime_graphs,
                )
                    .chain(),
            );
    }
}

#[derive(Asset, TypePath)]
pub struct AnimGraphProjectAsset {
    pub project: SavedAnimGraph,
}

#[derive(Component)]
pub struct AnimGraphProjectRuntime {
    pub project: Handle<AnimGraphProjectAsset>,
    pub gltf: Handle<Gltf>,
}

#[derive(Default, TypePath)]
pub struct AnimGraphProjectLoader;

impl AssetLoader for AnimGraphProjectLoader {
    type Asset = AnimGraphProjectAsset;
    type Settings = ();
    type Error = AnimGraphProjectLoadError;

    fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext,
    ) -> impl bevy::tasks::ConditionalSendFuture<Output = Result<Self::Asset, Self::Error>> {
        async move {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await?;
            let project = ron::de::from_bytes(&bytes)?;
            Ok(AnimGraphProjectAsset { project })
        }
    }

    fn extensions(&self) -> &[&str] {
        &["animgraph_editor.ron", "animgraph_project.ron"]
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AnimGraphProjectLoadError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Ron(#[from] ron::error::SpannedError),
}

#[derive(Component)]
pub struct AnimGraphRuntime {
    pub editor: AnimGraphEditor,
    pub gltf: Handle<Gltf>,
    pub graph: Option<Handle<AnimationGraph>>,
    pub playable_nodes: Vec<AnimationNodeIndex>,
    pub playable_names: Vec<String>,
    pub native_node_names: Vec<(AnimationNodeIndex, String)>,
    pub live_clips: Vec<LiveClipNode>,
    pub live_node_weights: Vec<LiveNodeWeight>,
    pub live_transitions: Vec<LiveTransition>,
    pub live_one_shots: Vec<LiveOneShot>,
    pub handled_completions: Vec<egui_graph_edit::NodeId>,
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
            native_node_names: Vec::new(),
            live_clips: Vec::new(),
            live_node_weights: Vec::new(),
            live_transitions: Vec::new(),
            live_one_shots: Vec::new(),
            handled_completions: Vec::new(),
            status: "Waiting for GLB".to_string(),
        }
    }

    pub fn request_rebuild(&mut self) {
        self.graph = None;
        self.playable_nodes.clear();
        self.playable_names.clear();
        self.native_node_names.clear();
        self.live_clips.clear();
        self.live_node_weights.clear();
        self.live_transitions.clear();
        self.live_one_shots.clear();
        self.handled_completions.clear();
        self.status = "Rebuild requested".to_string();
    }

    pub fn from_project(project: &AnimGraphProjectAsset, gltf: Handle<Gltf>) -> Self {
        let mut editor = AnimGraphEditor::default();
        editor.graph = project.project.graph.clone();
        editor.preview_output = project.project.preview_output;
        editor.ensure_state_inputs();
        editor.ensure_playback_inputs();
        editor.clamp_float_values();
        Self::new(editor, gltf)
    }

    pub fn set_float(&mut self, name: &str, value: f32) -> bool {
        self.editor.set_float_parameter(name, value)
    }

    pub fn set_bool(&mut self, name: &str, value: bool) -> bool {
        self.editor.set_bool_parameter(name, value)
    }

    pub fn trigger(&mut self, name: &str) -> bool {
        let changed = self.editor.trigger_parameter(name);
        if changed {
            self.handled_completions.clear();
            for animation in &mut self.live_transitions {
                if transition_condition_uses_trigger(&self.editor, animation.editor_node, name) {
                    animation.progress = 0.0;
                    animation.target = 1.0;
                }
            }
            for one_shot in &mut self.live_one_shots {
                if one_shot_condition_uses_trigger(&self.editor, one_shot.editor_node, name) {
                    one_shot.progress = 0.0;
                    one_shot.target = 1.0;
                    one_shot.restart_requested = true;
                }
            }
        }
        changed
    }
}

#[derive(Clone, Copy)]
pub struct LiveClipNode {
    pub editor_node: egui_graph_edit::NodeId,
    pub playback_node: egui_graph_edit::NodeId,
    pub playback_animation: AnimationNodeIndex,
    pub animation: AnimationNodeIndex,
}

pub fn apply_clip_playback(
    editor: &AnimGraphEditor,
    live_clip: LiveClipNode,
    active_animation: &mut ActiveAnimation,
    clip_duration: f32,
) {
    let settings = editor
        .playback_settings(live_clip.playback_node)
        .unwrap_or_default();
    let speed = clip_speed(editor, live_clip.editor_node) * settings.speed.max(0.0);
    let clip_duration = clip_duration.max(0.0);

    active_animation.set_weight(1.0);

    match settings.mode {
        PlaybackMode::Loop => {
            active_animation.repeat();
            active_animation.set_speed(speed);
            active_animation.resume();
        }
        PlaybackMode::Once => {
            active_animation.set_repeat(RepeatAnimation::Never);
            active_animation.set_speed(speed);
        }
        PlaybackMode::OnceHold => {
            active_animation.set_repeat(RepeatAnimation::Never);
            active_animation.set_speed(speed);
            if active_animation.is_finished() {
                active_animation.set_seek_time(clip_duration);
                active_animation.pause();
            }
        }
        PlaybackMode::PingPong => {
            active_animation.repeat();
            let direction = if active_animation.completions() % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            active_animation.set_speed(speed * direction);
            active_animation.resume();
        }
        PlaybackMode::PingPongOnce => {
            active_animation.set_repeat(RepeatAnimation::Count(2));
            let direction = if active_animation.completions() % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            active_animation.set_speed(speed * direction);
            if active_animation.is_finished() {
                active_animation.set_seek_time(0.0);
                active_animation.pause();
            }
        }
        PlaybackMode::Manual => {
            active_animation.set_repeat(RepeatAnimation::Forever);
            active_animation.set_speed(0.0);
            active_animation.set_seek_time(settings.start_offset_seconds.min(clip_duration));
            active_animation.pause();
        }
    }

    if settings.start_offset_seconds > 0.0
        && active_animation.elapsed() <= f32::EPSILON
        && !matches!(settings.mode, PlaybackMode::Manual)
    {
        active_animation.set_seek_time(settings.start_offset_seconds.min(clip_duration));
    }
}

pub fn apply_completion_actions(
    editor: &mut AnimGraphEditor,
    live_clips: &[LiveClipNode],
    handled_completions: &mut Vec<egui_graph_edit::NodeId>,
    player: &AnimationPlayer,
    graph: Option<&AnimationGraph>,
) -> Vec<String> {
    let mut events = Vec::new();

    for live_clip in live_clips {
        let playback_node = live_clip.playback_node;
        if handled_completions.contains(&playback_node) {
            continue;
        }
        if !live_clip_is_contributing(graph, *live_clip) {
            continue;
        }

        let Some(active_animation) = player.animation(live_clip.animation) else {
            continue;
        };
        if !active_animation.is_finished() {
            continue;
        }

        let Some(action) = editor.completion_action(playback_node) else {
            continue;
        };

        match action {
            CompletionAction::Stay => {}
            CompletionAction::TransitionTo(target) => {
                if request_state_transition(editor, playback_node, &target) {
                    events.push(format!(
                        "{} completed; transitioning to {target}",
                        state_name(editor, playback_node)
                    ));
                } else {
                    events.push(format!(
                        "{} completed; no transition to {target}",
                        state_name(editor, playback_node)
                    ));
                }
            }
            CompletionAction::SetBool { name, value } => {
                if set_bool_parameter(editor, &name, value) {
                    events.push(format!(
                        "{} completed; set {name}={value}",
                        state_name(editor, playback_node)
                    ));
                } else {
                    events.push(format!(
                        "{} completed; bool parameter {name} was not found",
                        state_name(editor, playback_node)
                    ));
                }
            }
            CompletionAction::EmitEvent(event) => {
                events.push(format!(
                    "{} completed; event {event}",
                    state_name(editor, playback_node)
                ));
            }
        }

        handled_completions.push(playback_node);
    }

    events
}

fn live_clip_is_contributing(graph: Option<&AnimationGraph>, live_clip: LiveClipNode) -> bool {
    graph
        .and_then(|graph| graph.get(live_clip.playback_animation))
        .map(|node| node.weight > 0.001)
        .unwrap_or(true)
}

#[derive(Clone, Copy)]
pub struct LiveNodeWeight {
    pub animation: AnimationNodeIndex,
    pub driver: WeightDriver,
}

#[derive(Clone, Copy)]
pub struct LiveTransition {
    pub editor_node: egui_graph_edit::NodeId,
    pub progress: f32,
    pub target: f32,
}

#[derive(Clone, Copy)]
pub struct LiveOneShot {
    pub editor_node: egui_graph_edit::NodeId,
    pub progress: f32,
    pub target: f32,
    pub restart_requested: bool,
}

#[derive(Clone, Copy)]
pub enum WeightDriver {
    BlendA(egui_graph_edit::NodeId),
    BlendB(egui_graph_edit::NodeId),
    WeightedBlendA(egui_graph_edit::NodeId),
    WeightedBlendB(egui_graph_edit::NodeId),
    TransitionFrom(egui_graph_edit::NodeId),
    TransitionTo(egui_graph_edit::NodeId),
    OneShotBase(egui_graph_edit::NodeId),
    OneShotAction(egui_graph_edit::NodeId),
}

impl WeightDriver {
    pub fn resolve(
        self,
        editor: &AnimGraphEditor,
        transitions: &[LiveTransition],
        one_shots: &[LiveOneShot],
    ) -> f32 {
        match self {
            Self::BlendA(node) => 1.0 - blend_weight(editor, node),
            Self::BlendB(node) => blend_weight(editor, node),
            Self::WeightedBlendA(node) => graph_weight_input(editor, node, "A Weight", 1.0),
            Self::WeightedBlendB(node) => graph_weight_input(editor, node, "B Weight", 1.0),
            Self::TransitionFrom(node) => 1.0 - transition_progress(editor, transitions, node),
            Self::TransitionTo(node) => transition_progress(editor, transitions, node),
            Self::OneShotBase(node) => 1.0 - one_shot_progress(editor, one_shots, node),
            Self::OneShotAction(node) => one_shot_progress(editor, one_shots, node),
        }
    }
}

pub struct CompiledEditorGraph {
    pub graph: AnimationGraph,
    pub playable_nodes: Vec<AnimationNodeIndex>,
    pub playable_names: Vec<String>,
    pub native_node_names: Vec<(AnimationNodeIndex, String)>,
    pub live_clips: Vec<LiveClipNode>,
    pub live_node_weights: Vec<LiveNodeWeight>,
    pub live_transitions: Vec<LiveTransition>,
    pub live_one_shots: Vec<LiveOneShot>,
    pub summary: String,
}

pub struct GraphValidation {
    pub can_apply: bool,
    pub message: String,
}

pub fn hydrate_project_runtimes(
    mut commands: Commands,
    projects: Res<Assets<AnimGraphProjectAsset>>,
    pending: Query<
        (Entity, &AnimGraphProjectRuntime),
        (With<AnimationPlayer>, Without<AnimGraphRuntime>),
    >,
) {
    for (entity, pending_runtime) in &pending {
        let Some(project) = projects.get(&pending_runtime.project) else {
            continue;
        };
        commands
            .entity(entity)
            .insert(AnimGraphRuntime::from_project(
                project,
                pending_runtime.gltf.clone(),
            ));
    }
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
                runtime.native_node_names = compiled.native_node_names;
                runtime.live_clips = compiled.live_clips;
                runtime.live_node_weights = compiled.live_node_weights;
                runtime.live_transitions = compiled.live_transitions;
                runtime.live_one_shots = compiled.live_one_shots;
                runtime.handled_completions.clear();
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
    time: Res<Time>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    animation_clips: Res<Assets<AnimationClip>>,
    mut runtimes: Query<(
        &mut AnimGraphRuntime,
        &AnimationGraphHandle,
        &mut AnimationPlayer,
    )>,
) {
    for (mut runtime, graph_handle, mut player) in &mut runtimes {
        let live_clips = runtime.live_clips.clone();
        let events = {
            let mut handled_completions = std::mem::take(&mut runtime.handled_completions);
            let graph = graphs.get(graph_handle);
            let events = apply_completion_actions(
                &mut runtime.editor,
                &live_clips,
                &mut handled_completions,
                &player,
                graph,
            );
            runtime.handled_completions = handled_completions;
            events
        };
        if let Some(event) = events.last() {
            runtime.status = event.clone();
        }

        let mut live_transitions_for_tick = std::mem::take(&mut runtime.live_transitions);
        let mut live_one_shots_for_tick = std::mem::take(&mut runtime.live_one_shots);
        tick_live_blends(
            &runtime.editor,
            &mut live_transitions_for_tick,
            &mut live_one_shots_for_tick,
            time.delta_secs(),
        );
        runtime.live_transitions = live_transitions_for_tick;
        runtime.live_one_shots = live_one_shots_for_tick;
        let live_transitions = runtime.live_transitions.clone();
        let live_one_shots = runtime.live_one_shots.clone();

        if let Some(graph) = graphs.get_mut(graph_handle) {
            sync_animation_graph_weights_with_transitions(
                &runtime.editor,
                graph,
                &runtime.live_node_weights,
                &live_transitions,
                &live_one_shots,
            );
        }

        let graph = runtime
            .graph
            .as_ref()
            .and_then(|graph_handle| graphs.get(graph_handle));
        let mut clip_durations = Vec::new();
        for live_clip in &runtime.live_clips {
            if !player.is_playing_animation(live_clip.animation) {
                player.play(live_clip.animation).repeat();
            }

            let Some(active_animation) = player.animation_mut(live_clip.animation) else {
                continue;
            };

            if runtime.live_one_shots.iter().any(|one_shot| {
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
            apply_clip_playback(&runtime.editor, *live_clip, active_animation, clip_duration);
        }

        let live_clips = runtime.live_clips.clone();
        update_one_shot_completion_targets(
            &live_clips,
            &mut runtime.live_one_shots,
            &player,
            &clip_durations,
        );
        for one_shot in &mut runtime.live_one_shots {
            one_shot.restart_requested = false;
        }

        if runtime.editor.consume_trigger_parameters() {
            runtime.handled_completions.clear();
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
    let mut native_node_names = Vec::new();
    let mut live_clips = Vec::new();
    let mut live_node_weights = Vec::new();
    let mut live_transitions = Vec::new();
    let mut live_one_shots = Vec::new();
    let root = graph.root;
    compile_source_node(
        editor,
        gltf,
        source,
        &mut graph,
        root,
        &mut playable_nodes,
        &mut playable_names,
        &mut native_node_names,
        &mut live_clips,
        &mut live_node_weights,
        &mut live_transitions,
        &mut live_one_shots,
        None,
        None,
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
        native_node_names,
        live_clips,
        live_node_weights,
        live_transitions,
        live_one_shots,
        summary,
    })
}

pub fn tick_animation_graph(
    editor: &AnimGraphEditor,
    graph: &mut AnimationGraph,
    live_node_weights: &[LiveNodeWeight],
    live_transitions: &mut [LiveTransition],
    live_one_shots: &mut [LiveOneShot],
    delta_secs: f32,
) {
    tick_live_blends(editor, live_transitions, live_one_shots, delta_secs);

    sync_animation_graph_weights_with_transitions(
        editor,
        graph,
        live_node_weights,
        live_transitions,
        live_one_shots,
    );
}

pub fn tick_live_blends(
    editor: &AnimGraphEditor,
    live_transitions: &mut [LiveTransition],
    live_one_shots: &mut [LiveOneShot],
    delta_secs: f32,
) {
    prime_triggered_transitions(editor, live_transitions);
    prime_triggered_one_shots(editor, live_one_shots);
    for transition in live_transitions.iter_mut() {
        *transition = advance_transition_progress(editor, *transition, delta_secs);
    }
    for one_shot in live_one_shots.iter_mut() {
        *one_shot = advance_one_shot_progress(editor, *one_shot, delta_secs);
    }
}

pub fn prime_triggered_transitions(
    editor: &AnimGraphEditor,
    live_transitions: &mut [LiveTransition],
) {
    for transition in live_transitions {
        if transition_condition_uses_any_trigger(editor, transition.editor_node)
            && transition_target(editor, transition.editor_node) >= 1.0
        {
            transition.progress = 0.0;
            transition.target = 1.0;
        }
    }
}

pub fn prime_triggered_one_shots(editor: &AnimGraphEditor, live_one_shots: &mut [LiveOneShot]) {
    for one_shot in live_one_shots {
        if one_shot_target(editor, one_shot.editor_node) >= 1.0 {
            one_shot.progress = 0.0;
            one_shot.target = 1.0;
            one_shot.restart_requested = true;
        }
    }
}

fn sync_animation_graph_weights_with_transitions(
    editor: &AnimGraphEditor,
    graph: &mut AnimationGraph,
    live_node_weights: &[LiveNodeWeight],
    live_transitions: &[LiveTransition],
    live_one_shots: &[LiveOneShot],
) {
    for live_weight in live_node_weights {
        if let Some(node) = graph.get_mut(live_weight.animation) {
            node.weight = live_weight
                .driver
                .resolve(editor, live_transitions, live_one_shots)
                .clamp(0.0, 1.0);
        }
    }
}

pub fn sync_animation_graph_weights(
    editor: &AnimGraphEditor,
    graph: &mut AnimationGraph,
    live_node_weights: &[LiveNodeWeight],
) {
    for live_weight in live_node_weights {
        if let Some(node) = graph.get_mut(live_weight.animation) {
            node.weight = live_weight.driver.resolve(editor, &[], &[]).clamp(0.0, 1.0);
        }
    }
}

pub fn update_one_shot_completion_targets(
    live_clips: &[LiveClipNode],
    live_one_shots: &mut [LiveOneShot],
    player: &AnimationPlayer,
    clip_durations: &[(AnimationNodeIndex, f32)],
) {
    for one_shot in live_one_shots {
        if one_shot.restart_requested {
            continue;
        }

        if one_shot.target < 1.0 || one_shot.progress < 0.999 {
            continue;
        }

        let mut action_clips = live_clips
            .iter()
            .filter(|live_clip| live_clip.playback_node == one_shot.editor_node)
            .peekable();
        if action_clips.peek().is_none() {
            continue;
        }

        if action_clips.all(|live_clip| one_shot_clip_completed(*live_clip, player, clip_durations))
        {
            one_shot.target = 0.0;
        }
    }
}

fn one_shot_clip_completed(
    live_clip: LiveClipNode,
    player: &AnimationPlayer,
    clip_durations: &[(AnimationNodeIndex, f32)],
) -> bool {
    let Some(animation) = player.animation(live_clip.animation) else {
        return false;
    };
    if !animation.is_finished() {
        return false;
    }

    let duration = clip_durations
        .iter()
        .find_map(|(animation, duration)| (*animation == live_clip.animation).then_some(*duration))
        .unwrap_or(0.0);

    if duration > f32::EPSILON {
        animation.elapsed() + 0.001 >= duration
    } else {
        animation.elapsed() > 0.001
    }
}

fn advance_transition_progress(
    editor: &AnimGraphEditor,
    transition: LiveTransition,
    delta_secs: f32,
) -> LiveTransition {
    let requested_target = transition_target(editor, transition.editor_node);
    let target = if transition_condition_uses_any_trigger(editor, transition.editor_node) {
        if requested_target >= 1.0 {
            1.0
        } else {
            transition.target
        }
    } else {
        requested_target
    };
    let duration = transition_duration(editor, transition.editor_node);

    if duration <= f32::EPSILON {
        return LiveTransition {
            target,
            progress: target,
            ..transition
        };
    }

    let step = (delta_secs / duration).clamp(0.0, 1.0);
    let progress = if transition.progress < target {
        (transition.progress + step).min(target)
    } else {
        (transition.progress - step).max(target)
    };

    LiveTransition {
        target,
        progress,
        ..transition
    }
}

fn advance_one_shot_progress(
    editor: &AnimGraphEditor,
    one_shot: LiveOneShot,
    delta_secs: f32,
) -> LiveOneShot {
    let duration = if one_shot.progress < one_shot.target {
        one_shot_fade_in(editor, one_shot.editor_node)
    } else {
        one_shot_fade_out(editor, one_shot.editor_node)
    };

    if duration <= f32::EPSILON {
        return LiveOneShot {
            progress: one_shot.target,
            ..one_shot
        };
    }

    let step = (delta_secs / duration).clamp(0.0, 1.0);
    let progress = if one_shot.progress < one_shot.target {
        (one_shot.progress + step).min(one_shot.target)
    } else {
        (one_shot.progress - step).max(one_shot.target)
    };

    LiveOneShot {
        progress,
        ..one_shot
    }
}

fn transition_progress(
    editor: &AnimGraphEditor,
    live_transitions: &[LiveTransition],
    node: egui_graph_edit::NodeId,
) -> f32 {
    live_transitions
        .iter()
        .find_map(|transition| {
            (transition.editor_node == node).then_some(transition.progress.clamp(0.0, 1.0))
        })
        .unwrap_or_else(|| transition_target(editor, node))
}

fn one_shot_progress(
    editor: &AnimGraphEditor,
    live_one_shots: &[LiveOneShot],
    node: egui_graph_edit::NodeId,
) -> f32 {
    live_one_shots
        .iter()
        .find_map(|one_shot| {
            (one_shot.editor_node == node).then_some(one_shot.progress.clamp(0.0, 1.0))
        })
        .unwrap_or_else(|| one_shot_target(editor, node))
}

fn transition_condition_uses_any_trigger(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
) -> bool {
    let Ok(input) = editor.graph.graph.nodes[node].get_input("Condition") else {
        return false;
    };
    let Some(output) = editor.graph.graph.connection(input) else {
        return false;
    };
    let source = editor.graph.graph.get_output(output).node;
    matches!(
        editor.graph.graph.nodes[source].user_data.template,
        AnimNodeTemplate::TriggerParameter
    )
}

fn transition_condition_uses_trigger(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    trigger_name: &str,
) -> bool {
    let Ok(input) = editor.graph.graph.nodes[node].get_input("Condition") else {
        return false;
    };
    let Some(output) = editor.graph.graph.connection(input) else {
        return false;
    };
    let source = editor.graph.graph.get_output(output).node;
    matches!(
        editor.graph.graph.nodes[source].user_data.template,
        AnimNodeTemplate::TriggerParameter
    ) && node_input_text(editor, source, "Name").as_deref() == Some(trigger_name)
}

fn one_shot_condition_uses_trigger(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    trigger_name: &str,
) -> bool {
    let Ok(input) = editor.graph.graph.nodes[node].get_input("Trigger") else {
        return false;
    };
    let Some(output) = editor.graph.graph.connection(input) else {
        return false;
    };
    let source = editor.graph.graph.get_output(output).node;
    matches!(
        editor.graph.graph.nodes[source].user_data.template,
        AnimNodeTemplate::TriggerParameter
    ) && node_input_text(editor, source, "Name").as_deref() == Some(trigger_name)
}

fn transition_target(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> f32 {
    if resolve_bool_input(editor, node, "Condition").unwrap_or(false) {
        1.0
    } else {
        0.0
    }
}

fn one_shot_target(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> f32 {
    if resolve_bool_input(editor, node, "Trigger").unwrap_or(false) {
        1.0
    } else {
        0.0
    }
}

fn transition_duration(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> f32 {
    resolve_float_input(editor, node, "Duration")
        .unwrap_or(0.2)
        .max(MIN_TRANSITION_DURATION)
}

fn one_shot_fade_in(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> f32 {
    resolve_float_input(editor, node, "Fade In")
        .unwrap_or(0.08)
        .max(MIN_TRANSITION_DURATION)
}

fn one_shot_fade_out(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> f32 {
    resolve_float_input(editor, node, "Fade Out")
        .unwrap_or(0.12)
        .max(MIN_TRANSITION_DURATION)
}

pub fn clip_speed(editor: &AnimGraphEditor, clip_node: egui_graph_edit::NodeId) -> f32 {
    node_input_float(editor, clip_node, "Speed")
        .unwrap_or(1.0)
        .max(0.0)
}

fn request_state_transition(
    editor: &mut AnimGraphEditor,
    from_state: egui_graph_edit::NodeId,
    target_state: &str,
) -> bool {
    let target_state = target_state.trim();
    if target_state.is_empty() {
        return false;
    }

    let transitions: Vec<_> = editor
        .graph
        .graph
        .nodes
        .iter()
        .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::Transition))
        .map(|(node_id, _)| node_id)
        .collect();

    for transition in transitions {
        if connected_pose_node(editor, transition, "From") != Some(from_state) {
            continue;
        }

        let to_state = connected_pose_node(editor, transition, "To")
            .map(|node| state_name(editor, node))
            .unwrap_or_default();
        if to_state == target_state {
            return set_transition_condition(editor, transition, true);
        }
    }

    false
}

fn set_bool_parameter(editor: &mut AnimGraphEditor, name: &str, value: bool) -> bool {
    let name = name.trim();
    let nodes: Vec<_> = editor
        .graph
        .graph
        .nodes
        .iter()
        .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::BoolParameter))
        .map(|(node_id, _)| node_id)
        .collect();

    let mut changed = false;
    for node in nodes {
        if node_input_text(editor, node, "Name").as_deref() == Some(name) {
            changed |= set_node_bool_value(editor, node, "Value", value);
        }
    }

    changed
}

fn set_transition_condition(
    editor: &mut AnimGraphEditor,
    transition: egui_graph_edit::NodeId,
    value: bool,
) -> bool {
    let Ok(input) = editor.graph.graph.nodes[transition].get_input("Condition") else {
        return false;
    };

    if let Some(output) = editor.graph.graph.connection(input) {
        let source = editor.graph.graph.get_output(output).node;
        if matches!(
            editor.graph.graph.nodes[source].user_data.template,
            AnimNodeTemplate::BoolParameter
        ) {
            return set_node_bool_value(editor, source, "Value", value);
        }
    }

    editor.graph.graph.inputs[input].value = AnimValue::Bool(value);
    true
}

fn set_node_bool_value(
    editor: &mut AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
    value: bool,
) -> bool {
    let Ok(input) = editor.graph.graph.nodes[node].get_input(input_name) else {
        return false;
    };
    editor.graph.graph.inputs[input].value = AnimValue::Bool(value);
    true
}

fn connected_pose_node(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
) -> Option<egui_graph_edit::NodeId> {
    let input = editor.graph.graph.nodes[node].get_input(input_name).ok()?;
    let output = editor.graph.graph.connection(input)?;
    Some(editor.graph.graph.get_output(output).node)
}

pub fn native_tree_lines(
    graph: &AnimationGraph,
    node_names: &[(AnimationNodeIndex, String)],
) -> Vec<String> {
    let mut lines = Vec::new();
    append_native_tree_lines(graph, graph.root, node_names, 0, &mut lines);
    lines
}

pub fn preview_tree_signature(editor: &AnimGraphEditor) -> Option<String> {
    let output = preview_output_node(editor)?;
    let input = editor.graph.graph.nodes[output].get_input("Pose").ok()?;
    let source = editor.graph.graph.connection(input)?;
    let mut signature = format!("output:{:?};", output);
    append_output_signature(editor, source, &mut signature);
    Some(signature)
}

fn append_native_tree_lines(
    graph: &AnimationGraph,
    node_index: AnimationNodeIndex,
    node_names: &[(AnimationNodeIndex, String)],
    depth: usize,
    lines: &mut Vec<String>,
) {
    let Some(node) = graph.get(node_index) else {
        return;
    };

    let indent = "  ".repeat(depth);
    let label = match &node.node_type {
        AnimationNodeType::Clip(_) => node_names
            .iter()
            .find_map(|(index, name)| (*index == node_index).then_some(name.as_str()))
            .unwrap_or("Clip"),
        AnimationNodeType::Blend => node_names
            .iter()
            .find_map(|(index, name)| (*index == node_index).then_some(name.as_str()))
            .unwrap_or("Blend"),
        AnimationNodeType::Add => "Add",
    };
    lines.push(format!(
        "{indent}{label} #{:?} weight {:.3}",
        node_index.index(),
        node.weight
    ));

    let mut children: Vec<_> = graph.graph.neighbors(node_index).collect();
    children.sort_by_key(|child| child.index());
    for child in children {
        append_native_tree_lines(graph, child, node_names, depth + 1, lines);
    }
}

fn compile_source_node(
    editor: &AnimGraphEditor,
    gltf: &Gltf,
    output: egui_graph_edit::OutputId,
    graph: &mut AnimationGraph,
    parent: AnimationNodeIndex,
    playable_nodes: &mut Vec<AnimationNodeIndex>,
    playable_names: &mut Vec<String>,
    native_node_names: &mut Vec<(AnimationNodeIndex, String)>,
    live_clips: &mut Vec<LiveClipNode>,
    live_node_weights: &mut Vec<LiveNodeWeight>,
    live_transitions: &mut Vec<LiveTransition>,
    live_one_shots: &mut Vec<LiveOneShot>,
    playback_node: Option<egui_graph_edit::NodeId>,
    playback_animation: Option<AnimationNodeIndex>,
    weight_driver: Option<WeightDriver>,
) -> Result<AnimationNodeIndex, String> {
    let node_id = editor.graph.graph.get_output(output).node;
    let node = &editor.graph.graph.nodes[node_id];
    let initial_weight = weight_driver
        .map(|driver| driver.resolve(editor, &[], &[]))
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);

    match node.user_data.template {
        AnimNodeTemplate::Clip => {
            let clip_label = node_input_text(editor, node_id, "Clip").unwrap_or_default();
            let (clip, clip_name) = resolve_clip(gltf, &clip_label)?;
            let node = graph.add_clip(clip, initial_weight, parent);
            native_node_names.push((node, clip_name.clone()));
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
                playback_node: playback_node.unwrap_or(node_id),
                playback_animation: playback_animation.unwrap_or(node),
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
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                playback_node,
                playback_animation,
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
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                playback_node,
                playback_animation,
                Some(WeightDriver::BlendB(node_id)),
            )?;
            Ok(blend)
        }
        AnimNodeTemplate::WeightedBlend => {
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
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                playback_node,
                playback_animation,
                Some(WeightDriver::WeightedBlendA(node_id)),
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
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                playback_node,
                playback_animation,
                Some(WeightDriver::WeightedBlendB(node_id)),
            )?;
            Ok(blend)
        }
        AnimNodeTemplate::OneShot => {
            let one_shot = graph.add_blend(initial_weight, parent);
            native_node_names.push((one_shot, format!("One Shot: {}", node.label)));
            live_one_shots.push(LiveOneShot {
                editor_node: node_id,
                progress: one_shot_target(editor, node_id),
                target: one_shot_target(editor, node_id),
                restart_requested: false,
            });
            if let Some(driver) = weight_driver {
                live_node_weights.push(LiveNodeWeight {
                    animation: one_shot,
                    driver,
                });
            }
            compile_connected_input(
                editor,
                gltf,
                node_id,
                "Base",
                graph,
                one_shot,
                playable_nodes,
                playable_names,
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                playback_node,
                playback_animation,
                Some(WeightDriver::OneShotBase(node_id)),
            )?;
            compile_connected_input(
                editor,
                gltf,
                node_id,
                "Action",
                graph,
                one_shot,
                playable_nodes,
                playable_names,
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                Some(node_id),
                Some(one_shot),
                Some(WeightDriver::OneShotAction(node_id)),
            )?;
            Ok(one_shot)
        }
        AnimNodeTemplate::State => {
            let state = graph.add_blend(initial_weight, parent);
            native_node_names.push((state, format!("State: {}", state_name(editor, node_id))));
            if let Some(driver) = weight_driver {
                live_node_weights.push(LiveNodeWeight {
                    animation: state,
                    driver,
                });
            }
            compile_connected_input(
                editor,
                gltf,
                node_id,
                "Pose",
                graph,
                state,
                playable_nodes,
                playable_names,
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                Some(node_id),
                Some(state),
                None,
            )?;
            Ok(state)
        }
        AnimNodeTemplate::Transition => {
            let transition = graph.add_blend(initial_weight, parent);
            live_transitions.push(LiveTransition {
                editor_node: node_id,
                progress: transition_target(editor, node_id),
                target: transition_target(editor, node_id),
            });
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
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                playback_node,
                playback_animation,
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
                native_node_names,
                live_clips,
                live_node_weights,
                live_transitions,
                live_one_shots,
                playback_node,
                playback_animation,
                Some(WeightDriver::TransitionTo(node_id)),
            )?;
            Ok(transition)
        }
        AnimNodeTemplate::FloatParameter
        | AnimNodeTemplate::BoolParameter
        | AnimNodeTemplate::TriggerParameter
        | AnimNodeTemplate::Remap
        | AnimNodeTemplate::Add
        | AnimNodeTemplate::Multiply
        | AnimNodeTemplate::Invert
        | AnimNodeTemplate::Clamp
        | AnimNodeTemplate::Smoothstep
        | AnimNodeTemplate::Compare
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
    native_node_names: &mut Vec<(AnimationNodeIndex, String)>,
    live_clips: &mut Vec<LiveClipNode>,
    live_node_weights: &mut Vec<LiveNodeWeight>,
    live_transitions: &mut Vec<LiveTransition>,
    live_one_shots: &mut Vec<LiveOneShot>,
    playback_node: Option<egui_graph_edit::NodeId>,
    playback_animation: Option<AnimationNodeIndex>,
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
        native_node_names,
        live_clips,
        live_node_weights,
        live_transitions,
        live_one_shots,
        playback_node,
        playback_animation,
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
        AnimNodeTemplate::WeightedBlend => {
            append_connected_signature(editor, node_id, "A", signature);
            append_connected_signature(editor, node_id, "B", signature);
        }
        AnimNodeTemplate::OneShot => {
            append_connected_signature(editor, node_id, "Base", signature);
            append_connected_signature(editor, node_id, "Action", signature);
        }
        AnimNodeTemplate::State => {
            signature.push_str("state:");
            signature.push_str(&state_name(editor, node_id));
            signature.push(';');
            append_connected_signature(editor, node_id, "Pose", signature);
        }
        AnimNodeTemplate::Transition => {
            append_connected_signature(editor, node_id, "From", signature);
            append_connected_signature(editor, node_id, "To", signature);
        }
        AnimNodeTemplate::FloatParameter
        | AnimNodeTemplate::BoolParameter
        | AnimNodeTemplate::TriggerParameter
        | AnimNodeTemplate::Remap
        | AnimNodeTemplate::Add
        | AnimNodeTemplate::Multiply
        | AnimNodeTemplate::Invert
        | AnimNodeTemplate::Clamp
        | AnimNodeTemplate::Smoothstep
        | AnimNodeTemplate::Compare
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

fn state_name(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> String {
    node_input_text(editor, node, "Name")
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| editor.graph.graph.nodes[node].label.clone())
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

fn graph_weight_input(
    editor: &AnimGraphEditor,
    node: egui_graph_edit::NodeId,
    input_name: &str,
    fallback: f32,
) -> f32 {
    resolve_float_input(editor, node, input_name)
        .unwrap_or(fallback)
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
    resolve_float_node(editor, node)
}

fn resolve_float_node(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> Option<f32> {
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
        AnimNodeTemplate::Add => Some(
            resolve_float_input(editor, node, "A").unwrap_or(0.0)
                + resolve_float_input(editor, node, "B").unwrap_or(0.0),
        ),
        AnimNodeTemplate::Multiply => Some(
            resolve_float_input(editor, node, "A").unwrap_or(0.0)
                * resolve_float_input(editor, node, "B").unwrap_or(1.0),
        ),
        AnimNodeTemplate::Invert => {
            Some(1.0 - resolve_float_input(editor, node, "Value").unwrap_or(0.0))
        }
        AnimNodeTemplate::Clamp => {
            let value = resolve_float_input(editor, node, "Value").unwrap_or(0.0);
            let min = node_input_float(editor, node, "Min").unwrap_or(0.0);
            let max = node_input_float(editor, node, "Max").unwrap_or(1.0);
            Some(if min <= max {
                value.clamp(min, max)
            } else {
                value.clamp(max, min)
            })
        }
        AnimNodeTemplate::Smoothstep => {
            let value = resolve_float_input(editor, node, "Value").unwrap_or(0.0);
            let edge0 = node_input_float(editor, node, "Edge 0").unwrap_or(0.0);
            let edge1 = node_input_float(editor, node, "Edge 1").unwrap_or(1.0);
            let range = edge1 - edge0;
            if range.abs() <= f32::EPSILON {
                Some(if value >= edge1 { 1.0 } else { 0.0 })
            } else {
                let t = ((value - edge0) / range).clamp(0.0, 1.0);
                Some(t * t * (3.0 - 2.0 * t))
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
    resolve_bool_node(editor, node)
}

fn resolve_bool_node(editor: &AnimGraphEditor, node: egui_graph_edit::NodeId) -> Option<bool> {
    match editor.graph.graph.nodes[node].user_data.template {
        AnimNodeTemplate::BoolParameter => node_input_bool(editor, node, "Value"),
        AnimNodeTemplate::TriggerParameter => node_input_bool(editor, node, "Value"),
        AnimNodeTemplate::Compare => {
            let value = resolve_float_input(editor, node, "Value").unwrap_or(0.0);
            let threshold = resolve_float_input(editor, node, "Threshold").unwrap_or(0.5);
            let mode = node_input_text(editor, node, "Mode").unwrap_or_else(|| ">=".to_string());
            Some(match mode.trim() {
                ">" => value > threshold,
                "<" => value < threshold,
                "<=" => value <= threshold,
                "==" => (value - threshold).abs() <= 0.0001,
                "!=" => (value - threshold).abs() > 0.0001,
                _ => value >= threshold,
            })
        }
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
