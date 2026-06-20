use std::{
    borrow::Cow,
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Color32};
use egui_graph_edit::{
    DataTypeTrait, Graph, GraphEditorState, InputId, InputParamKind, NodeDataTrait, NodeId,
    NodeResponse, NodeTemplateIter, NodeTemplateTrait, OutputId, UserResponseTrait,
    WidgetValueTrait,
};
use serde::{Deserialize, Serialize};

pub type EditorState =
    GraphEditorState<AnimNodeData, AnimDataType, AnimValue, AnimNodeTemplate, AnimGraphUiState>;

pub const MIN_TRANSITION_DURATION: f32 = 0.001;

#[derive(Resource)]
pub struct AnimGraphEditor {
    pub graph: EditorState,
    pub ui_state: AnimGraphUiState,
    pub templates: AnimNodeTemplates,
    pub preview_output: Option<NodeId>,
    pub current_project_path: Option<PathBuf>,
    pub last_event: String,
}

#[derive(Serialize, Deserialize)]
pub struct SavedAnimGraph {
    pub graph: EditorState,
    pub preview_output: Option<NodeId>,
    #[serde(default)]
    pub gltf_asset_path: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct RuntimeAnimGraph {
    pub version: u32,
    pub gltf_asset_path: Option<String>,
    pub preview_output: Option<usize>,
    pub nodes: Vec<RuntimeAnimNode>,
    pub connections: Vec<RuntimeAnimConnection>,
    pub state_machines: Vec<RuntimeStateMachine>,
}

#[derive(Serialize, Deserialize)]
pub struct RuntimeAnimNode {
    pub id: usize,
    pub label: String,
    pub template: AnimNodeTemplate,
    pub playback: Option<PlaybackSettings>,
    pub on_complete: Option<CompletionAction>,
    pub inputs: Vec<RuntimeAnimInput>,
    pub outputs: Vec<RuntimeAnimOutput>,
}

#[derive(Serialize, Deserialize)]
pub struct RuntimeAnimInput {
    pub name: String,
    pub typ: AnimDataType,
    pub value: AnimValue,
    pub kind: InputParamKind,
}

#[derive(Serialize, Deserialize)]
pub struct RuntimeAnimOutput {
    pub name: String,
    pub typ: AnimDataType,
}

#[derive(Serialize, Deserialize)]
pub struct RuntimeAnimConnection {
    pub from_node: usize,
    pub from_output: String,
    pub to_node: usize,
    pub to_input: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeStateMachine {
    pub name: String,
    pub initial_state: Option<usize>,
    pub states: Vec<RuntimeState>,
    pub transitions: Vec<RuntimeStateTransition>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeState {
    pub node: usize,
    pub name: String,
    pub pose_node: usize,
    pub playback: PlaybackSettings,
    pub on_complete: CompletionAction,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeStateTransition {
    pub node: usize,
    pub from: usize,
    pub to: usize,
    pub condition: ConditionExpression,
    pub duration_seconds: f32,
    pub interrupt: InterruptPolicy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackMode {
    #[default]
    Loop,
    Once,
    OnceHold,
    PingPong,
    PingPongOnce,
    Manual,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaybackSettings {
    pub mode: PlaybackMode,
    pub speed: f32,
    pub start_offset_seconds: f32,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            mode: PlaybackMode::Loop,
            speed: 1.0,
            start_offset_seconds: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CompletionAction {
    Stay,
    TransitionTo(String),
    SetBool { name: String, value: bool },
    EmitEvent(String),
}

impl Default for CompletionAction {
    fn default() -> Self {
        Self::Stay
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ConditionExpression {
    BoolParameter(String),
    Constant(bool),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptPolicy {
    #[default]
    None,
    HigherPriority,
    Any,
}

impl Default for AnimGraphEditor {
    fn default() -> Self {
        Self {
            graph: EditorState::new(1.0),
            ui_state: AnimGraphUiState::default(),
            templates: AnimNodeTemplates,
            preview_output: None,
            current_project_path: None,
            last_event: "Ready".to_string(),
        }
    }
}

impl AnimGraphEditor {
    pub fn selected_clip_node(&self) -> Option<NodeId> {
        self.graph.selected_nodes.first().copied().filter(|node| {
            self.graph
                .graph
                .nodes
                .get(*node)
                .is_some_and(|node| matches!(node.user_data.template, AnimNodeTemplate::Clip))
        })
    }

    pub fn set_clip_node_label(&mut self, node: NodeId, label: String) -> bool {
        let Some(graph_node) = self.graph.graph.nodes.get(node) else {
            return false;
        };
        if !matches!(graph_node.user_data.template, AnimNodeTemplate::Clip) {
            return false;
        }

        let Ok(input) = graph_node.get_input("Clip") else {
            return false;
        };
        self.graph.graph.inputs[input].value = AnimValue::Text(label);
        true
    }

    pub fn clip_node_label(&self, node: NodeId) -> Option<&str> {
        let graph_node = self.graph.graph.nodes.get(node)?;
        let input = graph_node.get_input("Clip").ok()?;
        match self.graph.graph.inputs[input].value() {
            AnimValue::Text(value) => Some(value.as_str()),
            _ => None,
        }
    }

    pub fn save_to_path(
        &self,
        path: impl AsRef<Path>,
        gltf_asset_path: Option<&str>,
    ) -> Result<(), SaveGraphError> {
        let saved = SavedAnimGraph {
            graph: self.graph.clone(),
            preview_output: self.preview_output,
            gltf_asset_path: gltf_asset_path.map(str::to_string),
        };
        let ron = ron::ser::to_string_pretty(&saved, ron::ser::PrettyConfig::default())?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, ron)?;
        Ok(())
    }

    pub fn save_runtime_graph_to_path(
        &self,
        path: impl AsRef<Path>,
        gltf_asset_path: Option<&str>,
    ) -> Result<(), SaveGraphError> {
        let runtime_graph = self.runtime_graph(gltf_asset_path);
        let ron = ron::ser::to_string_pretty(&runtime_graph, ron::ser::PrettyConfig::default())?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, ron)?;
        Ok(())
    }

    pub fn runtime_graph(&self, gltf_asset_path: Option<&str>) -> RuntimeAnimGraph {
        let node_ids: HashMap<NodeId, usize> = self
            .graph
            .graph
            .nodes
            .iter()
            .enumerate()
            .map(|(index, (node_id, _))| (node_id, index))
            .collect();

        let nodes = self
            .graph
            .graph
            .nodes
            .iter()
            .filter_map(|(node_id, node)| {
                let id = *node_ids.get(&node_id)?;
                Some(RuntimeAnimNode {
                    id,
                    label: node.label.clone(),
                    template: node.user_data.template,
                    playback: node_playback_settings(&self.graph.graph, node_id),
                    on_complete: node_completion_action(&self.graph.graph, node_id),
                    inputs: node
                        .inputs
                        .iter()
                        .map(|(name, input_id)| {
                            let input = self.graph.graph.get_input(*input_id);
                            RuntimeAnimInput {
                                name: name.clone(),
                                typ: input.typ.clone(),
                                value: input.value.clone(),
                                kind: input.kind,
                            }
                        })
                        .collect(),
                    outputs: node
                        .outputs
                        .iter()
                        .map(|(name, output_id)| {
                            let output = self.graph.graph.get_output(*output_id);
                            RuntimeAnimOutput {
                                name: name.clone(),
                                typ: output.typ.clone(),
                            }
                        })
                        .collect(),
                })
            })
            .collect();

        let connections = self
            .graph
            .graph
            .iter_connections()
            .filter_map(|(input_id, output_id)| {
                let input = self.graph.graph.get_input(input_id);
                let output = self.graph.graph.get_output(output_id);
                let to_node = *node_ids.get(&input.node)?;
                let from_node = *node_ids.get(&output.node)?;
                let to_input = self.graph.graph.nodes[input.node]
                    .inputs
                    .iter()
                    .find_map(|(name, id)| (*id == input_id).then(|| name.clone()))?;
                let from_output = self.graph.graph.nodes[output.node]
                    .outputs
                    .iter()
                    .find_map(|(name, id)| (*id == output_id).then(|| name.clone()))?;

                Some(RuntimeAnimConnection {
                    from_node,
                    from_output,
                    to_node,
                    to_input,
                })
            })
            .collect();

        RuntimeAnimGraph {
            version: 1,
            gltf_asset_path: gltf_asset_path.map(str::to_string),
            preview_output: self
                .preview_output
                .and_then(|node_id| node_ids.get(&node_id).copied()),
            nodes,
            connections,
            state_machines: self.runtime_state_machines(&node_ids),
        }
    }

    pub fn playback_settings(&self, node: NodeId) -> Option<PlaybackSettings> {
        node_playback_settings(&self.graph.graph, node)
    }

    pub fn completion_action(&self, node: NodeId) -> Option<CompletionAction> {
        node_completion_action(&self.graph.graph, node)
    }

    pub fn set_float_parameter(&mut self, name: &str, value: f32) -> bool {
        let nodes: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::FloatParameter))
            .map(|(node_id, _)| node_id)
            .collect();

        let mut changed = false;
        for node in nodes {
            if graph_node_input_text(&self.graph.graph, node, "Name") == Some(name) {
                changed |= self.set_node_input_value(node, "Value", AnimValue::Float(value));
            }
        }
        changed
    }

    pub fn set_bool_parameter(&mut self, name: &str, value: bool) -> bool {
        let nodes: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::BoolParameter))
            .map(|(node_id, _)| node_id)
            .collect();

        let mut changed = false;
        for node in nodes {
            if graph_node_input_text(&self.graph.graph, node, "Name") == Some(name) {
                changed |= self.set_node_input_value(node, "Value", AnimValue::Bool(value));
            }
        }
        changed
    }

    pub fn trigger_parameter(&mut self, name: &str) -> bool {
        let nodes: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| {
                matches!(node.user_data.template, AnimNodeTemplate::TriggerParameter)
            })
            .map(|(node_id, _)| node_id)
            .collect();

        let mut changed = false;
        for node in nodes {
            if graph_node_input_text(&self.graph.graph, node, "Name") == Some(name) {
                changed |= self.set_node_input_value(node, "Value", AnimValue::Bool(true));
            }
        }
        changed
    }

    pub fn consume_trigger_parameters(&mut self) -> bool {
        let nodes: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| {
                matches!(node.user_data.template, AnimNodeTemplate::TriggerParameter)
            })
            .map(|(node_id, _)| node_id)
            .collect();

        let mut consumed = false;
        for node in nodes {
            if graph_node_input_bool(&self.graph.graph, node, "Value").unwrap_or(false) {
                consumed = true;
            }
            self.set_node_input_value(node, "Value", AnimValue::Bool(false));
        }
        consumed
    }

    fn set_node_input_value(&mut self, node: NodeId, input_name: &str, value: AnimValue) -> bool {
        let Ok(input) = self.graph.graph.nodes[node].get_input(input_name) else {
            return false;
        };
        self.graph.graph.inputs[input].value = value;
        true
    }

    pub fn sync_node_labels(&mut self) {
        let labels: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter_map(|(node_id, node)| {
                let label = match node.user_data.template {
                    AnimNodeTemplate::Clip => self
                        .clip_node_label(node_id)
                        .map(display_clip_name)
                        .map(|clip| format!("Clip: {clip}")),
                    AnimNodeTemplate::FloatParameter => {
                        graph_node_input_text(&self.graph.graph, node_id, "Name")
                            .filter(|name| !name.trim().is_empty())
                            .map(|name| format!("Float: {}", name.trim()))
                    }
                    AnimNodeTemplate::BoolParameter => {
                        graph_node_input_text(&self.graph.graph, node_id, "Name")
                            .filter(|name| !name.trim().is_empty())
                            .map(|name| format!("Bool: {}", name.trim()))
                    }
                    _ => None,
                }?;
                Some((node_id, label))
            })
            .collect();

        for (node_id, label) in labels {
            self.graph.graph.nodes[node_id].label = label;
        }
    }

    fn runtime_state_machines(
        &self,
        node_ids: &HashMap<NodeId, usize>,
    ) -> Vec<RuntimeStateMachine> {
        let states: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::State))
            .filter_map(|(node_id, _node)| {
                let node_index = *node_ids.get(&node_id)?;
                let pose_node = self
                    .graph
                    .graph
                    .nodes
                    .get(node_id)
                    .and_then(|node| node.get_input("Pose").ok())
                    .and_then(|input| self.graph.graph.connection(input))
                    .and_then(|output| node_ids.get(&self.graph.graph.get_output(output).node))
                    .copied()
                    .unwrap_or(node_index);

                Some(RuntimeState {
                    node: node_index,
                    name: graph_state_name(&self.graph.graph, node_id),
                    pose_node,
                    playback: node_playback_settings(&self.graph.graph, node_id)
                        .unwrap_or_default(),
                    on_complete: node_completion_action(&self.graph.graph, node_id)
                        .unwrap_or_default(),
                })
            })
            .collect();

        if states.is_empty() {
            return Vec::new();
        }

        let state_nodes: HashMap<_, _> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::State))
            .filter_map(|(node_id, _)| node_ids.get(&node_id).map(|index| (node_id, *index)))
            .collect();

        let transitions = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::Transition))
            .filter_map(|(node_id, _)| {
                let node = *node_ids.get(&node_id)?;
                let from = connected_state_node(&self.graph.graph, node_id, "From", &state_nodes)?;
                let to = connected_state_node(&self.graph.graph, node_id, "To", &state_nodes)?;
                Some(RuntimeStateTransition {
                    node,
                    from,
                    to,
                    condition: transition_condition_expression(&self.graph.graph, node_id),
                    duration_seconds: graph_transition_duration(&self.graph.graph, node_id),
                    interrupt: node_interrupt_policy(&self.graph.graph, node_id),
                })
            })
            .collect();

        vec![RuntimeStateMachine {
            name: "Default".to_string(),
            initial_state: states.first().map(|state| state.node),
            states,
            transitions,
        }]
    }

    pub fn load_from_path(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Option<String>, LoadGraphError> {
        let path = path.as_ref();
        let ron = fs::read_to_string(path)?;
        let saved: SavedAnimGraph = ron::from_str(&ron)?;
        self.graph = saved.graph;
        self.preview_output = saved.preview_output;
        self.current_project_path = Some(path.to_path_buf());
        self.ensure_state_inputs();
        self.ensure_playback_inputs();
        self.clamp_float_values();
        self.last_event = "Graph loaded".to_string();
        Ok(saved.gltf_asset_path)
    }

    pub fn clamp_float_values(&mut self) {
        let transition_duration_inputs: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::Transition))
            .filter_map(|(_, node)| node.get_input("Duration").ok())
            .collect();
        let start_offset_inputs: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter_map(|(_, node)| node.get_input("Start Offset").ok())
            .collect();

        for (input_id, input) in self.graph.graph.inputs.iter_mut() {
            if let AnimValue::Float(value) = &mut input.value {
                if transition_duration_inputs.contains(&input_id) {
                    *value = value.max(MIN_TRANSITION_DURATION);
                } else if start_offset_inputs.contains(&input_id) {
                    *value = value.max(0.0);
                } else {
                    *value = value.clamp(0.0, 1.0);
                }
            }
        }
    }

    pub fn ensure_state_inputs(&mut self) {
        let state_nodes: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::State))
            .map(|(node_id, _)| node_id)
            .collect();

        for node_id in state_nodes {
            if self.graph.graph.nodes[node_id].get_input("Name").is_err() {
                let label = self.graph.graph.nodes[node_id].label.clone();
                self.graph.graph.add_input_param(
                    node_id,
                    "Name".to_string(),
                    AnimDataType::Pose,
                    AnimValue::Text(label),
                    InputParamKind::ConstantOnly,
                    true,
                );
            }
        }
    }

    pub fn ensure_playback_inputs(&mut self) {
        let nodes: Vec<_> = self
            .graph
            .graph
            .nodes
            .iter()
            .map(|(node_id, node)| (node_id, node.user_data.template))
            .collect();

        for (node_id, template) in nodes {
            match template {
                AnimNodeTemplate::Clip | AnimNodeTemplate::OneShot => {
                    self.ensure_text_input(node_id, "Playback", "Loop");
                    self.ensure_float_input(node_id, "Start Offset", 0.0);
                }
                AnimNodeTemplate::State => {
                    self.ensure_text_input(node_id, "Playback", "Loop");
                    self.ensure_float_input(node_id, "Start Offset", 0.0);
                    self.ensure_text_input(node_id, "On Complete", "Stay");
                }
                AnimNodeTemplate::Transition => {
                    self.ensure_text_input(node_id, "Interrupt", "None");
                }
                _ => {}
            }
        }
    }

    fn ensure_text_input(&mut self, node_id: NodeId, name: &str, value: &str) {
        if self.graph.graph.nodes[node_id].get_input(name).is_ok() {
            return;
        }
        self.graph.graph.add_input_param(
            node_id,
            name.to_string(),
            AnimDataType::Text,
            AnimValue::Text(value.to_string()),
            InputParamKind::ConstantOnly,
            true,
        );
    }

    fn ensure_float_input(&mut self, node_id: NodeId, name: &str, value: f32) {
        if self.graph.graph.nodes[node_id].get_input(name).is_ok() {
            return;
        }
        self.graph.graph.add_input_param(
            node_id,
            name.to_string(),
            AnimDataType::Float,
            AnimValue::Float(value),
            InputParamKind::ConstantOnly,
            true,
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SaveGraphError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Ron(#[from] ron::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum LoadGraphError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Ron(#[from] ron::error::SpannedError),
}

#[derive(Clone, Default)]
pub struct AnimGraphUiState {
    pub edge_visualization: EdgeVisualization,
    pub weight_header_saturation: bool,
    pub contribution_borders: bool,
    pub preview_output: Option<NodeId>,
    pub live_one_shot_progress: HashMap<NodeId, f32>,
    pub available_clips: Vec<String>,
    pub clip_picker: Option<ClipPickerState>,
    pub flow_phases: HashMap<(InputId, OutputId), f32>,
    pub flow_last_time: Option<f32>,
}

#[derive(Clone)]
pub struct ClipPickerState {
    pub node: NodeId,
    pub query: String,
    pub position: egui::Pos2,
    pub just_spawned: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EdgeVisualization {
    #[default]
    Marker,
    Flow,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnimGraphResponse {
    SetOutput(NodeId),
}

impl UserResponseTrait for AnimGraphResponse {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimDataType {
    Pose,
    Float,
    Bool,
    Text,
}

impl DataTypeTrait<AnimGraphUiState> for AnimDataType {
    fn data_type_color(&self, _user_state: &mut AnimGraphUiState) -> Color32 {
        match self {
            Self::Pose => Color32::from_rgb(91, 166, 255),
            Self::Float => Color32::from_rgb(93, 214, 146),
            Self::Bool => Color32::from_rgb(245, 176, 75),
            Self::Text => Color32::from_rgb(190, 159, 104),
        }
    }

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Self::Pose => "Pose",
            Self::Float => "Float",
            Self::Bool => "Bool",
            Self::Text => "Text",
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AnimValue {
    None,
    Float(f32),
    Bool(bool),
    Text(String),
}

impl Default for AnimValue {
    fn default() -> Self {
        Self::None
    }
}

impl WidgetValueTrait for AnimValue {
    type Response = AnimGraphResponse;
    type UserState = AnimGraphUiState;
    type NodeData = AnimNodeData;

    fn value_widget(
        &mut self,
        param_name: &str,
        node_id: NodeId,
        ui: &mut egui::Ui,
        user_state: &mut Self::UserState,
        node_data: &Self::NodeData,
    ) -> Vec<Self::Response> {
        match self {
            Self::Float(value) => {
                ui.horizontal(|ui| {
                    ui.label(param_name);
                    if matches!(node_data.template, AnimNodeTemplate::Transition)
                        && param_name == "Duration"
                    {
                        ui.add(egui::DragValue::new(value).speed(0.01));
                        *value = value.max(MIN_TRANSITION_DURATION);
                    } else if param_name == "Start Offset" {
                        ui.add(egui::DragValue::new(value).speed(0.01));
                        *value = value.max(0.0);
                    } else {
                        ui.add(egui::DragValue::new(value).speed(0.01).range(0.0..=1.0));
                        *value = value.clamp(0.0, 1.0);
                    }
                });
            }
            Self::Bool(value) => {
                if matches!(node_data.template, AnimNodeTemplate::TriggerParameter)
                    && param_name == "Value"
                {
                    ui.horizontal(|ui| {
                        ui.label(param_name);
                        if ui.button("Fire").clicked() {
                            *value = true;
                        }
                        if *value {
                            ui.monospace("pending");
                        }
                    });
                } else {
                    ui.checkbox(value, param_name);
                }
            }
            Self::Text(value) => {
                if matches!(node_data.template, AnimNodeTemplate::Clip) && param_name == "Clip" {
                    clip_value_widget(ui, node_id, value, user_state);
                } else if matches!(
                    node_data.template,
                    AnimNodeTemplate::Clip | AnimNodeTemplate::OneShot | AnimNodeTemplate::State
                ) && param_name == "Playback"
                {
                    combo_text(
                        ui,
                        param_name,
                        value,
                        &[
                            "Loop",
                            "Once",
                            "OnceHold",
                            "PingPong",
                            "PingPongOnce",
                            "Manual",
                        ],
                    );
                } else if matches!(node_data.template, AnimNodeTemplate::State)
                    && param_name == "On Complete"
                {
                    completion_action_text(ui, param_name, value);
                } else if matches!(node_data.template, AnimNodeTemplate::Transition)
                    && param_name == "Interrupt"
                {
                    combo_text(ui, param_name, value, &["None", "HigherPriority", "Any"]);
                } else if matches!(node_data.template, AnimNodeTemplate::Compare)
                    && param_name == "Mode"
                {
                    combo_text(ui, param_name, value, &[">=", ">", "<=", "<", "==", "!="]);
                } else {
                    ui.horizontal(|ui| {
                        ui.label(param_name);
                        ui.text_edit_singleline(value);
                    });
                }
            }
            Self::None => {
                ui.label(param_name);
            }
        }

        Vec::new()
    }
}

fn combo_text(ui: &mut egui::Ui, label: &str, value: &mut String, options: &[&str]) {
    ui.horizontal(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(ui.next_auto_id())
            .selected_text(value.as_str())
            .show_ui(ui, |ui| {
                for option in options {
                    ui.selectable_value(value, (*option).to_string(), *option);
                }
            });
    });
}

fn clip_value_widget(
    ui: &mut egui::Ui,
    node_id: NodeId,
    value: &mut String,
    user_state: &mut AnimGraphUiState,
) {
    ui.horizontal(|ui| {
        ui.label("Clip");
        ui.text_edit_singleline(value);
        let button = ui.button("...").on_hover_text("Search animation clips");
        if button.clicked() {
            user_state.clip_picker = Some(ClipPickerState {
                node: node_id,
                query: display_clip_name(value).to_string(),
                position: button.rect.right_bottom() + egui::vec2(6.0, 4.0),
                just_spawned: true,
            });
        }

        let Some(picker) = user_state.clip_picker.as_mut() else {
            return;
        };
        if picker.node != node_id {
            return;
        }

        let mut close_picker = false;
        let mut selected_clip = None;
        let clip_names = user_state.available_clips.clone();

        egui::Area::new(egui::Id::new(("clip_picker", node_id)))
            .order(egui::Order::Foreground)
            .current_pos(picker.position)
            .show(ui.ctx(), |ui| {
                let (clip, should_close) =
                    show_clip_finder(ui, picker, &clip_names, egui::Id::new(node_id));
                if let Some(clip) = clip {
                    selected_clip = Some(clip);
                    close_picker = true;
                }
                close_picker |= should_close;
            });

        if let Some(clip) = selected_clip {
            *value = clip;
        }
        if close_picker {
            user_state.clip_picker = None;
        }
    });
}

fn show_clip_finder(
    ui: &mut egui::Ui,
    picker: &mut ClipPickerState,
    clips: &[String],
    id: egui::Id,
) -> (Option<String>, bool) {
    let background_color;
    let text_color;

    if ui.visuals().dark_mode {
        background_color = egui::Color32::from_rgb(63, 63, 63);
        text_color = egui::Color32::from_rgb(254, 254, 254);
    } else {
        background_color = egui::Color32::from_rgb(254, 254, 254);
        text_color = egui::Color32::from_rgb(63, 63, 63);
    }

    ui.visuals_mut().widgets.noninteractive.fg_stroke = egui::Stroke::new(2.0, text_color);

    let frame = egui::Frame::dark_canvas(ui.style())
        .fill(background_color)
        .inner_margin(egui::vec2(5.0, 5.0));

    let mut selected_clip = None;
    let mut close = false;
    frame.show(ui, |ui| {
        ui.vertical(|ui| {
            let response = ui.text_edit_singleline(&mut picker.query);
            if picker.just_spawned {
                response.request_focus();
                picker.just_spawned = false;
            }

            let mut query_submit =
                response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
            let max_height = ui.input(|input| input.content_rect().height() * 0.5);
            let scroll_area_width = response.rect.width() - 30.0;
            let query = picker.query.trim().to_ascii_lowercase();

            egui::Frame::default()
                .inner_margin(egui::vec2(10.0, 10.0))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("clip_finder_scroll", id))
                        .min_scrolled_height(max_height)
                        .max_height(max_height)
                        .show(ui, |ui| {
                            ui.set_width(scroll_area_width.max(220.0));
                            let filtered_clips: Vec<_> = clips
                                .iter()
                                .filter(|clip| {
                                    query.is_empty()
                                        || clip.to_ascii_lowercase().contains(query.as_str())
                                })
                                .collect();

                            if filtered_clips.is_empty() {
                                ui.label("No matching clips");
                                return;
                            }

                            for clip in filtered_clips {
                                if ui.selectable_label(false, clip).clicked() {
                                    selected_clip = Some(clip.clone());
                                } else if query_submit {
                                    selected_clip = Some(clip.clone());
                                    query_submit = false;
                                }
                            }
                        });
                });

            if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
    });

    (selected_clip, close)
}

fn completion_action_text(ui: &mut egui::Ui, label: &str, value: &mut String) {
    let current_kind = if value.starts_with("TransitionTo:") {
        "TransitionTo"
    } else if value.starts_with("SetBool:") {
        "SetBool"
    } else if value.starts_with("EmitEvent:") {
        "EmitEvent"
    } else {
        "Stay"
    };

    ui.horizontal(|ui| {
        ui.label(label);
        let mut selected_kind = current_kind.to_string();
        egui::ComboBox::from_id_salt(ui.next_auto_id())
            .selected_text(selected_kind.as_str())
            .show_ui(ui, |ui| {
                for option in ["Stay", "TransitionTo", "SetBool", "EmitEvent"] {
                    ui.selectable_value(&mut selected_kind, option.to_string(), option);
                }
            });

        if selected_kind != current_kind {
            *value = match selected_kind.as_str() {
                "TransitionTo" => "TransitionTo:State".to_string(),
                "SetBool" => "SetBool:flag=true".to_string(),
                "EmitEvent" => "EmitEvent:Event".to_string(),
                _ => "Stay".to_string(),
            };
        }

        match selected_kind.as_str() {
            "TransitionTo" => edit_prefixed_value(ui, value, "TransitionTo:"),
            "SetBool" => edit_prefixed_value(ui, value, "SetBool:"),
            "EmitEvent" => edit_prefixed_value(ui, value, "EmitEvent:"),
            _ => {}
        }
    });
}

fn edit_prefixed_value(ui: &mut egui::Ui, value: &mut String, prefix: &str) {
    if !value.starts_with(prefix) {
        *value = prefix.to_string();
    }
    let mut suffix = value.strip_prefix(prefix).unwrap_or_default().to_string();
    if ui.text_edit_singleline(&mut suffix).changed() {
        *value = format!("{prefix}{suffix}");
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnimNodeData {
    pub template: AnimNodeTemplate,
    pub note: String,
}

impl NodeDataTrait for AnimNodeData {
    type Response = AnimGraphResponse;
    type UserState = AnimGraphUiState;
    type DataType = AnimDataType;
    type ValueType = AnimValue;

    fn bottom_ui(
        &self,
        ui: &mut egui::Ui,
        node_id: NodeId,
        graph: &Graph<Self, Self::DataType, Self::ValueType>,
        user_state: &mut Self::UserState,
    ) -> Vec<NodeResponse<Self::Response, Self>>
    where
        Self::Response: UserResponseTrait,
    {
        let mut responses = Vec::new();
        ui.separator();
        ui.label(&self.note);

        if self.template.produces_float() {
            let output = resolve_graph_float_node(graph, node_id, 0);
            let text = output
                .map(|value| format!("Output: {value:.3}"))
                .unwrap_or_else(|| "Output: unavailable".to_string());
            ui.monospace(text);
        }

        if matches!(self.template, AnimNodeTemplate::Compare) {
            let output = resolve_graph_bool_node(graph, node_id, 0);
            let text = output
                .map(|value| format!("Output: {value}"))
                .unwrap_or_else(|| "Output: unavailable".to_string());
            ui.monospace(text);
        }

        if self.template.produces_pose() {
            let weights = pose_node_weights(graph, node_id, &user_state.live_one_shot_progress);
            if weights.is_empty() {
                ui.monospace("Weight: unreachable");
            } else {
                for weight in weights {
                    ui.monospace(format!(
                        "Weight: {:.3}  Effective: {:.3}",
                        weight.node_weight, weight.effective_weight
                    ));
                }
            }
        }

        if matches!(self.template, AnimNodeTemplate::Blend) {
            let weight = graph_blend_weight(graph, node_id);
            ui.monospace(format!("A: {:.3}  B: {:.3}", 1.0 - weight, weight));
        }

        if matches!(self.template, AnimNodeTemplate::WeightedBlend) {
            let a = graph_weight_input(graph, node_id, "A Weight", 1.0);
            let b = graph_weight_input(graph, node_id, "B Weight", 1.0);
            let sum = a + b;
            let normalized_a = if sum > f32::EPSILON { a / sum } else { 0.0 };
            let normalized_b = if sum > f32::EPSILON { b / sum } else { 0.0 };
            ui.monospace(format!("A raw: {:.3}  B raw: {:.3}", a, b));
            ui.monospace(format!(
                "A norm: {:.3}  B norm: {:.3}",
                normalized_a, normalized_b
            ));
        }

        if matches!(self.template, AnimNodeTemplate::State) {
            ui.monospace(format!("State: {}", graph_state_name(graph, node_id)));
        }

        if matches!(self.template, AnimNodeTemplate::Transition) {
            let condition =
                resolve_graph_bool_input(graph, node_id, "Condition", 0).unwrap_or(false);
            let from = if condition { 0.0 } else { 1.0 };
            let duration = graph_transition_duration(graph, node_id);
            ui.monospace(format!("Target From: {:.3}  To: {:.3}", from, 1.0 - from));
            ui.monospace(format!("Duration: {duration:.3}s"));
        }

        if matches!(self.template, AnimNodeTemplate::Output)
            && ui.button("Use as preview output").clicked()
        {
            responses.push(NodeResponse::User(AnimGraphResponse::SetOutput(node_id)));
        }

        responses
    }

    fn titlebar_color(
        &self,
        _ui: &egui::Ui,
        node_id: NodeId,
        graph: &Graph<Self, Self::DataType, Self::ValueType>,
        user_state: &mut Self::UserState,
    ) -> Option<Color32> {
        let base = match self.template {
            AnimNodeTemplate::Clip => Color32::from_rgb(47, 96, 146),
            AnimNodeTemplate::Blend
            | AnimNodeTemplate::WeightedBlend
            | AnimNodeTemplate::OneShot => Color32::from_rgb(61, 126, 93),
            AnimNodeTemplate::State => Color32::from_rgb(119, 91, 151),
            AnimNodeTemplate::Transition => Color32::from_rgb(143, 99, 55),
            AnimNodeTemplate::FloatParameter
            | AnimNodeTemplate::BoolParameter
            | AnimNodeTemplate::TriggerParameter => Color32::from_rgb(87, 105, 122),
            AnimNodeTemplate::Remap
            | AnimNodeTemplate::Add
            | AnimNodeTemplate::Multiply
            | AnimNodeTemplate::Invert
            | AnimNodeTemplate::Clamp
            | AnimNodeTemplate::Smoothstep
            | AnimNodeTemplate::Compare => Color32::from_rgb(102, 118, 73),
            AnimNodeTemplate::Output => Color32::from_rgb(140, 68, 84),
        };

        Some(if user_state.weight_header_saturation {
            saturate_by_value(base, node_header_value(graph, node_id, user_state))
        } else {
            base
        })
    }

    fn border_color(
        &self,
        _ui: &egui::Ui,
        node_id: NodeId,
        graph: &Graph<Self, Self::DataType, Self::ValueType>,
        user_state: &mut Self::UserState,
    ) -> Option<Color32> {
        if !user_state.contribution_borders {
            return None;
        }

        let contribution = node_contribution_value(
            graph,
            node_id,
            user_state.preview_output,
            &user_state.live_one_shot_progress,
        );
        if contribution <= 0.001 {
            return None;
        }

        let alpha = (35.0 + contribution.clamp(0.0, 1.0) * 220.0) as u8;
        Some(Color32::from_rgba_unmultiplied(126, 236, 255, alpha))
    }
}

fn node_header_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node_id: NodeId,
    user_state: &AnimGraphUiState,
) -> f32 {
    let node = &graph.nodes[node_id];
    match node.user_data.template {
        AnimNodeTemplate::Clip
        | AnimNodeTemplate::Blend
        | AnimNodeTemplate::WeightedBlend
        | AnimNodeTemplate::OneShot
        | AnimNodeTemplate::State
        | AnimNodeTemplate::Transition => {
            pose_node_weights(graph, node_id, &user_state.live_one_shot_progress)
                .into_iter()
                .map(|weight| weight.effective_weight)
                .fold(0.0, f32::max)
        }
        AnimNodeTemplate::FloatParameter => graph_node_input_float(graph, node_id, "Value")
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        AnimNodeTemplate::BoolParameter => {
            if graph_node_input_bool(graph, node_id, "Value").unwrap_or(false) {
                1.0
            } else {
                0.0
            }
        }
        AnimNodeTemplate::TriggerParameter => {
            if graph_node_input_bool(graph, node_id, "Value").unwrap_or(false) {
                1.0
            } else {
                0.0
            }
        }
        AnimNodeTemplate::Remap
        | AnimNodeTemplate::Add
        | AnimNodeTemplate::Multiply
        | AnimNodeTemplate::Invert
        | AnimNodeTemplate::Clamp
        | AnimNodeTemplate::Smoothstep => {
            resolve_graph_float_node(graph, node_id, 0).unwrap_or(0.0)
        }
        AnimNodeTemplate::Compare => {
            if resolve_graph_bool_node(graph, node_id, 0).unwrap_or(false) {
                1.0
            } else {
                0.0
            }
        }
        AnimNodeTemplate::Output => graph
            .nodes
            .get(node_id)
            .and_then(|node| node.get_input("Pose").ok())
            .and_then(|input| graph.connection(input))
            .map(|_| 1.0)
            .unwrap_or(0.0),
    }
}

fn node_contribution_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node_id: NodeId,
    preview_output: Option<NodeId>,
    live_one_shot_progress: &HashMap<NodeId, f32>,
) -> f32 {
    if matches!(
        graph.nodes[node_id].user_data.template,
        AnimNodeTemplate::Output
    ) {
        return if preview_output.is_none_or(|output| output == node_id)
            && output_pose_source(graph, Some(node_id)).is_some()
        {
            1.0
        } else {
            0.0
        };
    }

    pose_node_weights_for_output(graph, node_id, preview_output, live_one_shot_progress)
        .into_iter()
        .map(|weight| weight.effective_weight)
        .fold(0.0, f32::max)
}

fn saturate_by_value(color: Color32, value: f32) -> Color32 {
    let value = value.clamp(0.0, 1.0);
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let luminance = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32).round() as u8;
    let mix = |channel: u8| egui::lerp(luminance as f32..=channel as f32, value).round() as u8;
    Color32::from_rgba_unmultiplied(mix(r), mix(g), mix(b), a)
}

#[derive(Clone, Copy)]
struct PoseNodeWeight {
    node_weight: f32,
    effective_weight: f32,
}

fn pose_node_weights(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    target: NodeId,
    live_one_shot_progress: &HashMap<NodeId, f32>,
) -> Vec<PoseNodeWeight> {
    pose_node_weights_for_output(graph, target, None, live_one_shot_progress)
}

fn pose_node_weights_for_output(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    target: NodeId,
    preview_output: Option<NodeId>,
    live_one_shot_progress: &HashMap<NodeId, f32>,
) -> Vec<PoseNodeWeight> {
    let mut weights = Vec::new();
    for output in output_pose_sources(graph, preview_output) {
        collect_pose_node_weights(
            graph,
            output,
            target,
            1.0,
            1.0,
            &mut weights,
            live_one_shot_progress,
            0,
        );
    }
    weights
}

fn output_pose_sources(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    preview_output: Option<NodeId>,
) -> Vec<egui_graph_edit::OutputId> {
    if let Some(output_node) = preview_output {
        return output_pose_source(graph, Some(output_node))
            .into_iter()
            .collect();
    }

    graph
        .nodes
        .iter()
        .filter(|(_, node)| matches!(node.user_data.template, AnimNodeTemplate::Output))
        .filter_map(|(output_node, _)| output_pose_source(graph, Some(output_node)))
        .collect()
}

fn output_pose_source(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    output_node: Option<NodeId>,
) -> Option<egui_graph_edit::OutputId> {
    let output_node = output_node?;
    let input = graph.nodes[output_node].get_input("Pose").ok()?;
    graph.connection(input)
}

fn collect_pose_node_weights(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    output: egui_graph_edit::OutputId,
    target: NodeId,
    node_weight: f32,
    effective_weight: f32,
    weights: &mut Vec<PoseNodeWeight>,
    live_one_shot_progress: &HashMap<NodeId, f32>,
    depth: usize,
) {
    if depth > 64 {
        return;
    }

    let node_id = graph.get_output(output).node;
    let node = &graph.nodes[node_id];
    if node_id == target {
        weights.push(PoseNodeWeight {
            node_weight,
            effective_weight,
        });
    }

    match node.user_data.template {
        AnimNodeTemplate::Clip => {}
        AnimNodeTemplate::Blend => {
            let blend_weight = graph_blend_weight(graph, node_id);
            collect_weighted_pose_input(
                graph,
                node_id,
                "A",
                1.0 - blend_weight,
                1.0 - blend_weight,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
            collect_weighted_pose_input(
                graph,
                node_id,
                "B",
                blend_weight,
                blend_weight,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
        }
        AnimNodeTemplate::WeightedBlend => {
            let a_weight = graph_weight_input(graph, node_id, "A Weight", 1.0);
            let b_weight = graph_weight_input(graph, node_id, "B Weight", 1.0);
            let sum = a_weight + b_weight;
            let a_effective = if sum > f32::EPSILON {
                a_weight / sum
            } else {
                0.0
            };
            let b_effective = if sum > f32::EPSILON {
                b_weight / sum
            } else {
                0.0
            };
            collect_weighted_pose_input(
                graph,
                node_id,
                "A",
                a_weight,
                a_effective,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
            collect_weighted_pose_input(
                graph,
                node_id,
                "B",
                b_weight,
                b_effective,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
        }
        AnimNodeTemplate::OneShot => {
            let action = one_shot_visual_progress(graph, node_id, live_one_shot_progress);
            collect_weighted_pose_input(
                graph,
                node_id,
                "Base",
                1.0 - action,
                1.0 - action,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
            collect_weighted_pose_input(
                graph,
                node_id,
                "Action",
                action,
                action,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
        }
        AnimNodeTemplate::State => {
            collect_weighted_pose_input(
                graph,
                node_id,
                "Pose",
                node_weight,
                node_weight,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
        }
        AnimNodeTemplate::Transition => {
            let condition =
                resolve_graph_bool_input(graph, node_id, "Condition", 0).unwrap_or(false);
            let from = if condition { 0.0 } else { 1.0 };
            collect_weighted_pose_input(
                graph,
                node_id,
                "From",
                from,
                from,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
            collect_weighted_pose_input(
                graph,
                node_id,
                "To",
                1.0 - from,
                1.0 - from,
                effective_weight,
                target,
                weights,
                live_one_shot_progress,
                depth + 1,
            );
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

fn collect_weighted_pose_input(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    input_name: &str,
    child_weight: f32,
    child_effective_weight: f32,
    parent_effective_weight: f32,
    target: NodeId,
    weights: &mut Vec<PoseNodeWeight>,
    live_one_shot_progress: &HashMap<NodeId, f32>,
    depth: usize,
) {
    let Ok(input) = graph.nodes[node].get_input(input_name) else {
        return;
    };
    let Some(output) = graph.connection(input) else {
        return;
    };
    collect_pose_node_weights(
        graph,
        output,
        target,
        child_weight,
        parent_effective_weight * child_effective_weight,
        weights,
        live_one_shot_progress,
        depth,
    );
}

pub fn connection_marker_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    input: InputId,
    output: OutputId,
    live_one_shot_progress: &HashMap<NodeId, f32>,
) -> Option<f32> {
    let input_param = graph.get_input(input);
    match &input_param.typ {
        AnimDataType::Float => resolve_graph_float_output(graph, output, 0).map(|value| {
            if value.is_finite() {
                value.clamp(0.0, 1.0)
            } else {
                0.0
            }
        }),
        AnimDataType::Bool => {
            resolve_graph_bool_output(graph, output, 0).map(|value| if value { 1.0 } else { 0.0 })
        }
        AnimDataType::Pose => pose_connection_marker_value(graph, input, live_one_shot_progress),
        AnimDataType::Text => None,
    }
}

pub fn connection_contribution_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    input: InputId,
    preview_output: Option<NodeId>,
    live_one_shot_progress: &HashMap<NodeId, f32>,
) -> f32 {
    let input_param = graph.get_input(input);
    let node_contribution = node_contribution_value(
        graph,
        input_param.node,
        preview_output,
        live_one_shot_progress,
    )
    .clamp(0.0, 1.0);
    if node_contribution <= 0.001 {
        return 0.0;
    }

    match input_param.typ {
        AnimDataType::Pose => pose_connection_marker_value(graph, input, live_one_shot_progress)
            .map(|value| value * node_contribution)
            .unwrap_or(node_contribution),
        AnimDataType::Float | AnimDataType::Bool | AnimDataType::Text => node_contribution,
    }
}

fn pose_connection_marker_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    input: InputId,
    live_one_shot_progress: &HashMap<NodeId, f32>,
) -> Option<f32> {
    let input_param = graph.get_input(input);
    let node_id = input_param.node;
    let input_name = graph.nodes[node_id]
        .inputs
        .iter()
        .find_map(|(name, id)| (*id == input).then_some(name.as_str()))?;

    match graph.nodes[node_id].user_data.template {
        AnimNodeTemplate::Blend => match input_name {
            "A" => Some(1.0 - graph_blend_weight(graph, node_id)),
            "B" => Some(graph_blend_weight(graph, node_id)),
            _ => None,
        },
        AnimNodeTemplate::OneShot => {
            let action = one_shot_visual_progress(graph, node_id, live_one_shot_progress);
            match input_name {
                "Base" => Some(1.0 - action),
                "Action" => Some(action),
                _ => None,
            }
        }
        AnimNodeTemplate::WeightedBlend => {
            let a = graph_weight_input(graph, node_id, "A Weight", 1.0);
            let b = graph_weight_input(graph, node_id, "B Weight", 1.0);
            let sum = a + b;
            if sum <= f32::EPSILON {
                return Some(0.0);
            }

            match input_name {
                "A" => Some(a / sum),
                "B" => Some(b / sum),
                _ => None,
            }
        }
        AnimNodeTemplate::Transition => {
            let condition =
                resolve_graph_bool_input(graph, node_id, "Condition", 0).unwrap_or(false);
            match input_name {
                "From" => Some(if condition { 0.0 } else { 1.0 }),
                "To" => Some(if condition { 1.0 } else { 0.0 }),
                _ => None,
            }
        }
        AnimNodeTemplate::Output => (input_name == "Pose").then_some(1.0),
        _ => None,
    }
}

fn one_shot_visual_progress(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    live_one_shot_progress: &HashMap<NodeId, f32>,
) -> f32 {
    live_one_shot_progress
        .get(&node)
        .copied()
        .unwrap_or_else(|| {
            if resolve_graph_bool_input(graph, node, "Trigger", 0).unwrap_or(false) {
                1.0
            } else {
                0.0
            }
        })
        .clamp(0.0, 1.0)
}

fn graph_blend_weight(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node: NodeId) -> f32 {
    resolve_graph_float_input(graph, node, "Weight", 0)
        .unwrap_or(0.5)
        .clamp(0.0, 1.0)
}

fn graph_weight_input(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    input_name: &str,
    fallback: f32,
) -> f32 {
    resolve_graph_float_input(graph, node, input_name, 0)
        .unwrap_or(fallback)
        .clamp(0.0, 1.0)
}

fn graph_transition_duration(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> f32 {
    resolve_graph_float_input(graph, node, "Duration", 0)
        .unwrap_or(0.2)
        .max(MIN_TRANSITION_DURATION)
}

fn remap_node_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    value: f32,
) -> Option<f32> {
    let in_min = graph_node_input_float(graph, node, "In Min").unwrap_or(0.0);
    let in_max = graph_node_input_float(graph, node, "In Max").unwrap_or(1.0);
    let range = in_max - in_min;

    if range.abs() <= f32::EPSILON {
        Some(if value >= in_max { 1.0 } else { 0.0 })
    } else {
        Some(((value - in_min) / range).clamp(0.0, 1.0))
    }
}

fn add_node_value(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node: NodeId) -> f32 {
    resolve_graph_float_input(graph, node, "A", 0).unwrap_or(0.0)
        + resolve_graph_float_input(graph, node, "B", 0).unwrap_or(0.0)
}

fn multiply_node_value(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node: NodeId) -> f32 {
    resolve_graph_float_input(graph, node, "A", 0).unwrap_or(0.0)
        * resolve_graph_float_input(graph, node, "B", 0).unwrap_or(1.0)
}

fn invert_node_value(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node: NodeId) -> f32 {
    1.0 - resolve_graph_float_input(graph, node, "Value", 0).unwrap_or(0.0)
}

fn clamp_node_value(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node: NodeId) -> f32 {
    let value = resolve_graph_float_input(graph, node, "Value", 0).unwrap_or(0.0);
    let min = graph_node_input_float(graph, node, "Min").unwrap_or(0.0);
    let max = graph_node_input_float(graph, node, "Max").unwrap_or(1.0);
    if min <= max {
        value.clamp(min, max)
    } else {
        value.clamp(max, min)
    }
}

fn smoothstep_node_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> f32 {
    let value = resolve_graph_float_input(graph, node, "Value", 0).unwrap_or(0.0);
    let edge0 = graph_node_input_float(graph, node, "Edge 0").unwrap_or(0.0);
    let edge1 = graph_node_input_float(graph, node, "Edge 1").unwrap_or(1.0);
    let range = edge1 - edge0;
    if range.abs() <= f32::EPSILON {
        return if value >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((value - edge0) / range).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn compare_node_value(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node: NodeId) -> bool {
    let value = resolve_graph_float_input(graph, node, "Value", 0).unwrap_or(0.0);
    let threshold = resolve_graph_float_input(graph, node, "Threshold", 0).unwrap_or(0.5);
    let mode = graph_node_input_text(graph, node, "Mode").unwrap_or(">=");
    match mode.trim() {
        ">" => value > threshold,
        "<" => value < threshold,
        "<=" => value <= threshold,
        "==" => (value - threshold).abs() <= 0.0001,
        "!=" => (value - threshold).abs() > 0.0001,
        _ => value >= threshold,
    }
}

fn resolve_graph_float_input(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    input_name: &str,
    depth: usize,
) -> Option<f32> {
    let input = graph.nodes[node].get_input(input_name).ok()?;
    if let Some(output) = graph.connection(input) {
        return resolve_graph_float_output(graph, output, depth + 1);
    }

    graph_node_input_float(graph, node, input_name)
}

fn resolve_graph_float_output(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    output: egui_graph_edit::OutputId,
    depth: usize,
) -> Option<f32> {
    if depth > 32 {
        return None;
    }

    let node = graph.get_output(output).node;
    resolve_graph_float_node(graph, node, depth)
}

fn resolve_graph_float_node(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    depth: usize,
) -> Option<f32> {
    if depth > 32 {
        return None;
    }

    match graph.nodes[node].user_data.template {
        AnimNodeTemplate::FloatParameter => graph_node_input_float(graph, node, "Value"),
        AnimNodeTemplate::Remap => {
            let value = resolve_graph_float_input(graph, node, "Value", depth + 1)?;
            remap_node_value(graph, node, value)
        }
        AnimNodeTemplate::Add => Some(add_node_value(graph, node)),
        AnimNodeTemplate::Multiply => Some(multiply_node_value(graph, node)),
        AnimNodeTemplate::Invert => Some(invert_node_value(graph, node)),
        AnimNodeTemplate::Clamp => Some(clamp_node_value(graph, node)),
        AnimNodeTemplate::Smoothstep => Some(smoothstep_node_value(graph, node)),
        _ => None,
    }
}

fn resolve_graph_bool_input(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    input_name: &str,
    depth: usize,
) -> Option<bool> {
    let input = graph.nodes[node].get_input(input_name).ok()?;
    if let Some(output) = graph.connection(input) {
        return resolve_graph_bool_output(graph, output, depth + 1);
    }

    graph_node_input_bool(graph, node, input_name)
}

fn resolve_graph_bool_output(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    output: egui_graph_edit::OutputId,
    depth: usize,
) -> Option<bool> {
    if depth > 32 {
        return None;
    }

    let node = graph.get_output(output).node;
    resolve_graph_bool_node(graph, node, depth)
}

fn resolve_graph_bool_node(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    depth: usize,
) -> Option<bool> {
    if depth > 32 {
        return None;
    }

    match graph.nodes[node].user_data.template {
        AnimNodeTemplate::BoolParameter => graph_node_input_bool(graph, node, "Value"),
        AnimNodeTemplate::TriggerParameter => graph_node_input_bool(graph, node, "Value"),
        AnimNodeTemplate::Compare => Some(compare_node_value(graph, node)),
        _ => None,
    }
}

fn graph_node_input_float(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    input_name: &str,
) -> Option<f32> {
    let input = graph.nodes[node].get_input(input_name).ok()?;
    match graph.get_input(input).value() {
        AnimValue::Float(value) => Some(*value),
        _ => None,
    }
}

fn graph_node_input_bool(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    input_name: &str,
) -> Option<bool> {
    let input = graph.nodes[node].get_input(input_name).ok()?;
    match graph.get_input(input).value() {
        AnimValue::Bool(value) => Some(*value),
        _ => None,
    }
}

fn graph_node_input_text<'a>(
    graph: &'a Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
    input_name: &str,
) -> Option<&'a str> {
    let input = graph.nodes[node].get_input(input_name).ok()?;
    match graph.get_input(input).value() {
        AnimValue::Text(value) => Some(value.as_str()),
        _ => None,
    }
}

fn graph_state_name(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node: NodeId) -> String {
    graph_node_input_text(graph, node, "Name")
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| graph.nodes[node].label.clone())
}

fn display_clip_name(label: &str) -> &str {
    label
        .rsplit_once('#')
        .map(|(_, name)| name)
        .unwrap_or(label)
        .trim()
}

fn node_playback_settings(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> Option<PlaybackSettings> {
    let template = graph.nodes[node].user_data.template;
    if !matches!(
        template,
        AnimNodeTemplate::Clip | AnimNodeTemplate::OneShot | AnimNodeTemplate::State
    ) {
        return None;
    }

    Some(PlaybackSettings {
        mode: graph_node_input_text(graph, node, "Playback")
            .and_then(parse_playback_mode)
            .unwrap_or_default(),
        speed: graph_node_input_float(graph, node, "Speed").unwrap_or(1.0),
        start_offset_seconds: graph_node_input_float(graph, node, "Start Offset")
            .unwrap_or(0.0)
            .max(0.0),
    })
}

fn node_completion_action(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> Option<CompletionAction> {
    matches!(
        graph.nodes[node].user_data.template,
        AnimNodeTemplate::State
    )
    .then(|| {
        graph_node_input_text(graph, node, "On Complete")
            .map(parse_completion_action)
            .unwrap_or_default()
    })
}

fn node_interrupt_policy(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> InterruptPolicy {
    graph_node_input_text(graph, node, "Interrupt")
        .and_then(parse_interrupt_policy)
        .unwrap_or_default()
}

fn transition_condition_expression(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> ConditionExpression {
    let Ok(input) = graph.nodes[node].get_input("Condition") else {
        return ConditionExpression::Constant(false);
    };

    if let Some(output) = graph.connection(input) {
        let source = graph.get_output(output).node;
        if matches!(
            graph.nodes[source].user_data.template,
            AnimNodeTemplate::BoolParameter | AnimNodeTemplate::TriggerParameter
        ) {
            return ConditionExpression::BoolParameter(
                graph_node_input_text(graph, source, "Name")
                    .unwrap_or("condition")
                    .to_string(),
            );
        }
    }

    ConditionExpression::Constant(graph_node_input_bool(graph, node, "Condition").unwrap_or(false))
}

fn connected_state_node(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    transition: NodeId,
    input_name: &str,
    state_nodes: &HashMap<NodeId, usize>,
) -> Option<usize> {
    let input = graph.nodes[transition].get_input(input_name).ok()?;
    let output = graph.connection(input)?;
    let node = graph.get_output(output).node;
    state_nodes.get(&node).copied()
}

fn parse_playback_mode(value: &str) -> Option<PlaybackMode> {
    match value.trim() {
        "Loop" => Some(PlaybackMode::Loop),
        "Once" => Some(PlaybackMode::Once),
        "OnceHold" | "Once Hold" => Some(PlaybackMode::OnceHold),
        "PingPong" | "Ping Pong" => Some(PlaybackMode::PingPong),
        "PingPongOnce" | "Ping Pong Once" => Some(PlaybackMode::PingPongOnce),
        "Manual" => Some(PlaybackMode::Manual),
        _ => None,
    }
}

fn parse_completion_action(value: &str) -> CompletionAction {
    let value = value.trim();
    if value == "Stay" || value.is_empty() {
        CompletionAction::Stay
    } else if let Some(target) = value.strip_prefix("TransitionTo:") {
        CompletionAction::TransitionTo(target.trim().to_string())
    } else if let Some(event) = value.strip_prefix("EmitEvent:") {
        CompletionAction::EmitEvent(event.trim().to_string())
    } else if let Some(rest) = value.strip_prefix("SetBool:") {
        let (name, value) = rest
            .split_once('=')
            .map(|(name, value)| (name.trim(), value.trim()))
            .unwrap_or((rest.trim(), "true"));
        CompletionAction::SetBool {
            name: name.to_string(),
            value: value.eq_ignore_ascii_case("true") || value == "1",
        }
    } else {
        CompletionAction::EmitEvent(value.to_string())
    }
}

fn parse_interrupt_policy(value: &str) -> Option<InterruptPolicy> {
    match value.trim() {
        "None" => Some(InterruptPolicy::None),
        "HigherPriority" | "Higher Priority" => Some(InterruptPolicy::HigherPriority),
        "Any" => Some(InterruptPolicy::Any),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimNodeTemplate {
    Clip,
    Blend,
    WeightedBlend,
    OneShot,
    State,
    Transition,
    FloatParameter,
    BoolParameter,
    TriggerParameter,
    Remap,
    Add,
    Multiply,
    Invert,
    Clamp,
    Smoothstep,
    Compare,
    Output,
}

impl NodeTemplateTrait for AnimNodeTemplate {
    type NodeData = AnimNodeData;
    type DataType = AnimDataType;
    type ValueType = AnimValue;
    type UserState = AnimGraphUiState;
    type CategoryType = &'static str;

    fn node_finder_label(&self, _user_state: &mut Self::UserState) -> Cow<'_, str> {
        Cow::Borrowed(self.label())
    }

    fn node_finder_categories(&self, _user_state: &mut Self::UserState) -> Vec<Self::CategoryType> {
        vec![match self {
            Self::Clip | Self::Blend | Self::WeightedBlend | Self::OneShot => "Pose",
            Self::State | Self::Transition => "State Machine",
            Self::FloatParameter | Self::BoolParameter | Self::TriggerParameter => "Parameters",
            Self::Remap
            | Self::Add
            | Self::Multiply
            | Self::Invert
            | Self::Clamp
            | Self::Smoothstep
            | Self::Compare => "Math",
            Self::Output => "Output",
        }]
    }

    fn node_graph_label(&self, _user_state: &mut Self::UserState) -> String {
        self.label().to_string()
    }

    fn user_data(&self, _user_state: &mut Self::UserState) -> Self::NodeData {
        AnimNodeData {
            template: *self,
            note: self.note().to_string(),
        }
    }

    fn build_node(
        &self,
        graph: &mut Graph<Self::NodeData, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
        node_id: NodeId,
    ) {
        match self {
            Self::Clip => {
                graph.add_input_param(
                    node_id,
                    "Clip".to_string(),
                    AnimDataType::Pose,
                    AnimValue::Text("walk.glb#Walk".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Speed".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Playback".to_string(),
                    AnimDataType::Text,
                    AnimValue::Text("Loop".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Start Offset".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Pose".to_string(), AnimDataType::Pose);
            }
            Self::Blend => {
                graph.add_input_param(
                    node_id,
                    "A".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "B".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Weight".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.5),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_output_param(node_id, "Pose".to_string(), AnimDataType::Pose);
            }
            Self::WeightedBlend => {
                graph.add_input_param(
                    node_id,
                    "A".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "B".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "A Weight".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "B Weight".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_output_param(node_id, "Pose".to_string(), AnimDataType::Pose);
            }
            Self::OneShot => {
                graph.add_input_param(
                    node_id,
                    "Base".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Action".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Trigger".to_string(),
                    AnimDataType::Bool,
                    AnimValue::Bool(false),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Fade In".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.08),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Fade Out".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.12),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Playback".to_string(),
                    AnimDataType::Text,
                    AnimValue::Text("OnceHold".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Start Offset".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Pose".to_string(), AnimDataType::Pose);
            }
            Self::State => {
                graph.add_input_param(
                    node_id,
                    "Name".to_string(),
                    AnimDataType::Pose,
                    AnimValue::Text("State".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Pose".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Playback".to_string(),
                    AnimDataType::Text,
                    AnimValue::Text("Loop".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Start Offset".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "On Complete".to_string(),
                    AnimDataType::Text,
                    AnimValue::Text("Stay".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Pose".to_string(), AnimDataType::Pose);
            }
            Self::Transition => {
                graph.add_input_param(
                    node_id,
                    "From".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "To".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Condition".to_string(),
                    AnimDataType::Bool,
                    AnimValue::Bool(false),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Duration".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.2),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Interrupt".to_string(),
                    AnimDataType::Text,
                    AnimValue::Text("None".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Pose".to_string(), AnimDataType::Pose);
            }
            Self::FloatParameter => {
                graph.add_input_param(
                    node_id,
                    "Name".to_string(),
                    AnimDataType::Float,
                    AnimValue::Text("speed".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.5),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Float);
            }
            Self::BoolParameter => {
                graph.add_input_param(
                    node_id,
                    "Name".to_string(),
                    AnimDataType::Bool,
                    AnimValue::Text("grounded".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Bool,
                    AnimValue::Bool(false),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Bool);
            }
            Self::TriggerParameter => {
                graph.add_input_param(
                    node_id,
                    "Name".to_string(),
                    AnimDataType::Text,
                    AnimValue::Text("attack".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Bool,
                    AnimValue::Bool(false),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Bool);
            }
            Self::Remap => {
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "In Min".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "In Max".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Float);
            }
            Self::Add => {
                graph.add_input_param(
                    node_id,
                    "A".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "B".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Float);
            }
            Self::Multiply => {
                graph.add_input_param(
                    node_id,
                    "A".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "B".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Float);
            }
            Self::Invert => {
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Float);
            }
            Self::Clamp => {
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Min".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Max".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Float);
            }
            Self::Smoothstep => {
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Edge 0".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Edge 1".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(1.0),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Float);
            }
            Self::Compare => {
                graph.add_input_param(
                    node_id,
                    "Value".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.0),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Threshold".to_string(),
                    AnimDataType::Float,
                    AnimValue::Float(0.5),
                    InputParamKind::ConnectionOrConstant,
                    true,
                );
                graph.add_input_param(
                    node_id,
                    "Mode".to_string(),
                    AnimDataType::Text,
                    AnimValue::Text(">=".to_string()),
                    InputParamKind::ConstantOnly,
                    true,
                );
                graph.add_output_param(node_id, "Value".to_string(), AnimDataType::Bool);
            }
            Self::Output => {
                graph.add_input_param(
                    node_id,
                    "Pose".to_string(),
                    AnimDataType::Pose,
                    AnimValue::None,
                    InputParamKind::ConnectionOnly,
                    true,
                );
            }
        }
    }
}

impl AnimNodeTemplate {
    fn produces_pose(self) -> bool {
        matches!(
            self,
            Self::Clip
                | Self::Blend
                | Self::WeightedBlend
                | Self::OneShot
                | Self::State
                | Self::Transition
        )
    }

    fn produces_float(self) -> bool {
        matches!(
            self,
            Self::FloatParameter
                | Self::Remap
                | Self::Add
                | Self::Multiply
                | Self::Invert
                | Self::Clamp
                | Self::Smoothstep
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::Clip => "Animation Clip",
            Self::Blend => "Blend",
            Self::WeightedBlend => "Weighted Blend",
            Self::OneShot => "One Shot",
            Self::State => "State",
            Self::Transition => "Transition",
            Self::FloatParameter => "Float Parameter",
            Self::BoolParameter => "Bool Parameter",
            Self::TriggerParameter => "Trigger Parameter",
            Self::Remap => "Remap 0..1",
            Self::Add => "Add",
            Self::Multiply => "Multiply",
            Self::Invert => "Invert 1-x",
            Self::Clamp => "Clamp",
            Self::Smoothstep => "Smoothstep",
            Self::Compare => "Compare",
            Self::Output => "Output",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Clip => "Samples a Bevy animation clip.",
            Self::Blend => "Interpolates two poses by weight.",
            Self::WeightedBlend => "Bevy-style blend with raw child weights.",
            Self::OneShot => "Plays an action pose over a base pose, then returns automatically.",
            Self::State => "Names a pose-producing state.",
            Self::Transition => "Chooses between poses with a condition.",
            Self::FloatParameter => "Reads a numeric graph parameter.",
            Self::BoolParameter => "Reads a boolean graph parameter.",
            Self::TriggerParameter => "One-shot boolean parameter consumed after evaluation.",
            Self::Remap => "Maps a float range to a normalized 0..1 value.",
            Self::Add => "Adds two float values.",
            Self::Multiply => "Multiplies two float values.",
            Self::Invert => "Outputs one minus the input value.",
            Self::Clamp => "Constrains a float between min and max.",
            Self::Smoothstep => "Eases a value between two edges.",
            Self::Compare => "Converts a float comparison into a bool.",
            Self::Output => "Final pose used by the preview.",
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct AnimNodeTemplates;

impl NodeTemplateIter for AnimNodeTemplates {
    type Item = AnimNodeTemplate;

    fn all_kinds(&self) -> Vec<Self::Item> {
        vec![
            AnimNodeTemplate::Clip,
            AnimNodeTemplate::Blend,
            AnimNodeTemplate::WeightedBlend,
            AnimNodeTemplate::OneShot,
            AnimNodeTemplate::State,
            AnimNodeTemplate::Transition,
            AnimNodeTemplate::FloatParameter,
            AnimNodeTemplate::BoolParameter,
            AnimNodeTemplate::TriggerParameter,
            AnimNodeTemplate::Remap,
            AnimNodeTemplate::Add,
            AnimNodeTemplate::Multiply,
            AnimNodeTemplate::Invert,
            AnimNodeTemplate::Clamp,
            AnimNodeTemplate::Smoothstep,
            AnimNodeTemplate::Compare,
            AnimNodeTemplate::Output,
        ]
    }
}
