use std::{borrow::Cow, collections::HashMap, fs, io, path::Path};

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
    pub last_event: String,
}

#[derive(Serialize, Deserialize)]
pub struct SavedAnimGraph {
    pub graph: EditorState,
    pub preview_output: Option<NodeId>,
    #[serde(default)]
    pub gltf_asset_path: Option<String>,
}

impl Default for AnimGraphEditor {
    fn default() -> Self {
        Self {
            graph: EditorState::new(1.0),
            ui_state: AnimGraphUiState::default(),
            templates: AnimNodeTemplates,
            preview_output: None,
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

    pub fn load_from_path(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<Option<String>, LoadGraphError> {
        let ron = fs::read_to_string(path)?;
        let saved: SavedAnimGraph = ron::from_str(&ron)?;
        self.graph = saved.graph;
        self.preview_output = saved.preview_output;
        self.ensure_state_inputs();
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

        for (input_id, input) in self.graph.graph.inputs.iter_mut() {
            if let AnimValue::Float(value) = &mut input.value {
                if transition_duration_inputs.contains(&input_id) {
                    *value = value.max(MIN_TRANSITION_DURATION);
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
    pub flow_phases: HashMap<(InputId, OutputId), f32>,
    pub flow_last_time: Option<f32>,
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
}

impl DataTypeTrait<AnimGraphUiState> for AnimDataType {
    fn data_type_color(&self, _user_state: &mut AnimGraphUiState) -> Color32 {
        match self {
            Self::Pose => Color32::from_rgb(91, 166, 255),
            Self::Float => Color32::from_rgb(93, 214, 146),
            Self::Bool => Color32::from_rgb(245, 176, 75),
        }
    }

    fn name(&self) -> Cow<'_, str> {
        Cow::Borrowed(match self {
            Self::Pose => "Pose",
            Self::Float => "Float",
            Self::Bool => "Bool",
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
        _node_id: NodeId,
        ui: &mut egui::Ui,
        _user_state: &mut Self::UserState,
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
                    } else {
                        ui.add(egui::DragValue::new(value).speed(0.01).range(0.0..=1.0));
                        *value = value.clamp(0.0, 1.0);
                    }
                });
            }
            Self::Bool(value) => {
                ui.checkbox(value, param_name);
            }
            Self::Text(value) => {
                ui.horizontal(|ui| {
                    ui.label(param_name);
                    ui.text_edit_singleline(value);
                });
            }
            Self::None => {
                ui.label(param_name);
            }
        }

        Vec::new()
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
        _user_state: &mut Self::UserState,
    ) -> Vec<NodeResponse<Self::Response, Self>>
    where
        Self::Response: UserResponseTrait,
    {
        let mut responses = Vec::new();
        ui.separator();
        ui.label(&self.note);

        if matches!(self.template, AnimNodeTemplate::Remap) {
            let output = resolve_graph_float_input(graph, node_id, "Value", 0)
                .and_then(|value| remap_node_value(graph, node_id, value));
            let text = output
                .map(|value| format!("Output: {value:.3}"))
                .unwrap_or_else(|| "Output: unavailable".to_string());
            ui.monospace(text);
        }

        if self.template.produces_pose() {
            let weights = pose_node_weights(graph, node_id);
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
            AnimNodeTemplate::Blend | AnimNodeTemplate::WeightedBlend => {
                Color32::from_rgb(61, 126, 93)
            }
            AnimNodeTemplate::State => Color32::from_rgb(119, 91, 151),
            AnimNodeTemplate::Transition => Color32::from_rgb(143, 99, 55),
            AnimNodeTemplate::FloatParameter | AnimNodeTemplate::BoolParameter => {
                Color32::from_rgb(87, 105, 122)
            }
            AnimNodeTemplate::Remap => Color32::from_rgb(102, 118, 73),
            AnimNodeTemplate::Output => Color32::from_rgb(140, 68, 84),
        };

        Some(if user_state.weight_header_saturation {
            saturate_by_value(base, node_header_value(graph, node_id))
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

        let contribution = node_contribution_value(graph, node_id, user_state.preview_output);
        if contribution <= 0.001 {
            return None;
        }

        let alpha = (35.0 + contribution.clamp(0.0, 1.0) * 220.0) as u8;
        Some(Color32::from_rgba_unmultiplied(126, 236, 255, alpha))
    }
}

fn node_header_value(graph: &Graph<AnimNodeData, AnimDataType, AnimValue>, node_id: NodeId) -> f32 {
    let node = &graph.nodes[node_id];
    match node.user_data.template {
        AnimNodeTemplate::Clip
        | AnimNodeTemplate::Blend
        | AnimNodeTemplate::WeightedBlend
        | AnimNodeTemplate::State
        | AnimNodeTemplate::Transition => pose_node_weights(graph, node_id)
            .into_iter()
            .map(|weight| weight.effective_weight)
            .fold(0.0, f32::max),
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
        AnimNodeTemplate::Remap => resolve_graph_float_input(graph, node_id, "Value", 0)
            .and_then(|value| remap_node_value(graph, node_id, value))
            .unwrap_or(0.0),
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

    pose_node_weights_for_output(graph, node_id, preview_output)
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
) -> Vec<PoseNodeWeight> {
    pose_node_weights_for_output(graph, target, None)
}

fn pose_node_weights_for_output(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    target: NodeId,
    preview_output: Option<NodeId>,
) -> Vec<PoseNodeWeight> {
    let mut weights = Vec::new();
    for output in output_pose_sources(graph, preview_output) {
        collect_pose_node_weights(graph, output, target, 1.0, 1.0, &mut weights, 0);
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
                depth + 1,
            );
        }
        AnimNodeTemplate::FloatParameter
        | AnimNodeTemplate::BoolParameter
        | AnimNodeTemplate::Remap
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
        depth,
    );
}

pub fn connection_marker_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    input: InputId,
    output: OutputId,
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
        AnimDataType::Pose => pose_connection_marker_value(graph, input),
    }
}

pub fn connection_contribution_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    input: InputId,
    preview_output: Option<NodeId>,
) -> f32 {
    let input_param = graph.get_input(input);
    let node_contribution =
        node_contribution_value(graph, input_param.node, preview_output).clamp(0.0, 1.0);
    if node_contribution <= 0.001 {
        return 0.0;
    }

    match input_param.typ {
        AnimDataType::Pose => pose_connection_marker_value(graph, input)
            .map(|value| value * node_contribution)
            .unwrap_or(node_contribution),
        AnimDataType::Float | AnimDataType::Bool => node_contribution,
    }
}

fn pose_connection_marker_value(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    input: InputId,
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
    match graph.nodes[node].user_data.template {
        AnimNodeTemplate::FloatParameter => graph_node_input_float(graph, node, "Value"),
        AnimNodeTemplate::Remap => {
            let value = resolve_graph_float_input(graph, node, "Value", depth + 1)?;
            remap_node_value(graph, node, value)
        }
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
    match graph.nodes[node].user_data.template {
        AnimNodeTemplate::BoolParameter => graph_node_input_bool(graph, node, "Value"),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimNodeTemplate {
    Clip,
    Blend,
    WeightedBlend,
    State,
    Transition,
    FloatParameter,
    BoolParameter,
    Remap,
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
            Self::Clip | Self::Blend | Self::WeightedBlend => "Pose",
            Self::State | Self::Transition => "State Machine",
            Self::FloatParameter | Self::BoolParameter => "Parameters",
            Self::Remap => "Math",
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
            Self::Clip | Self::Blend | Self::WeightedBlend | Self::State | Self::Transition
        )
    }

    fn label(self) -> &'static str {
        match self {
            Self::Clip => "Animation Clip",
            Self::Blend => "Blend",
            Self::WeightedBlend => "Weighted Blend",
            Self::State => "State",
            Self::Transition => "Transition",
            Self::FloatParameter => "Float Parameter",
            Self::BoolParameter => "Bool Parameter",
            Self::Remap => "Remap 0..1",
            Self::Output => "Output",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Clip => "Samples a Bevy animation clip.",
            Self::Blend => "Interpolates two poses by weight.",
            Self::WeightedBlend => "Bevy-style blend with raw child weights.",
            Self::State => "Names a pose-producing state.",
            Self::Transition => "Chooses between poses with a condition.",
            Self::FloatParameter => "Reads a numeric graph parameter.",
            Self::BoolParameter => "Reads a boolean graph parameter.",
            Self::Remap => "Maps a float range to a normalized 0..1 value.",
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
            AnimNodeTemplate::State,
            AnimNodeTemplate::Transition,
            AnimNodeTemplate::FloatParameter,
            AnimNodeTemplate::BoolParameter,
            AnimNodeTemplate::Remap,
            AnimNodeTemplate::Output,
        ]
    }
}
