use std::{borrow::Cow, fs, io, path::Path};

use bevy::prelude::Resource;
use bevy_egui::egui::{self, Color32};
use egui_graph_edit::{
    DataTypeTrait, Graph, GraphEditorState, InputParamKind, NodeDataTrait, NodeId, NodeResponse,
    NodeTemplateIter, NodeTemplateTrait, UserResponseTrait, WidgetValueTrait,
};
use serde::{Deserialize, Serialize};

pub type EditorState =
    GraphEditorState<AnimNodeData, AnimDataType, AnimValue, AnimNodeTemplate, AnimGraphUiState>;

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
}

impl Default for AnimGraphEditor {
    fn default() -> Self {
        Self {
            graph: sample_graph(),
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

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), SaveGraphError> {
        let saved = SavedAnimGraph {
            graph: self.graph.clone(),
            preview_output: self.preview_output,
        };
        let ron = ron::ser::to_string_pretty(&saved, ron::ser::PrettyConfig::default())?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, ron)?;
        Ok(())
    }

    pub fn load_from_path(&mut self, path: impl AsRef<Path>) -> Result<(), LoadGraphError> {
        let ron = fs::read_to_string(path)?;
        let saved: SavedAnimGraph = ron::from_str(&ron)?;
        self.graph = saved.graph;
        self.preview_output = saved.preview_output;
        self.clamp_float_values();
        self.last_event = "Graph loaded".to_string();
        Ok(())
    }

    pub fn clamp_float_values(&mut self) {
        for (_, input) in self.graph.graph.inputs.iter_mut() {
            if let AnimValue::Float(value) = &mut input.value {
                *value = value.clamp(0.0, 1.0);
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
pub struct AnimGraphUiState;

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
        _node_data: &Self::NodeData,
    ) -> Vec<Self::Response> {
        match self {
            Self::Float(value) => {
                ui.horizontal(|ui| {
                    ui.label(param_name);
                    ui.add(egui::DragValue::new(value).speed(0.01).range(0.0..=1.0));
                    *value = value.clamp(0.0, 1.0);
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
        _graph: &Graph<Self, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
    ) -> Vec<NodeResponse<Self::Response, Self>>
    where
        Self::Response: UserResponseTrait,
    {
        let mut responses = Vec::new();
        ui.separator();
        ui.label(&self.note);

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
        _node_id: NodeId,
        _graph: &Graph<Self, Self::DataType, Self::ValueType>,
        _user_state: &mut Self::UserState,
    ) -> Option<Color32> {
        Some(match self.template {
            AnimNodeTemplate::Clip => Color32::from_rgb(47, 96, 146),
            AnimNodeTemplate::Blend => Color32::from_rgb(61, 126, 93),
            AnimNodeTemplate::State => Color32::from_rgb(119, 91, 151),
            AnimNodeTemplate::Transition => Color32::from_rgb(143, 99, 55),
            AnimNodeTemplate::FloatParameter | AnimNodeTemplate::BoolParameter => {
                Color32::from_rgb(87, 105, 122)
            }
            AnimNodeTemplate::Output => Color32::from_rgb(140, 68, 84),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnimNodeTemplate {
    Clip,
    Blend,
    State,
    Transition,
    FloatParameter,
    BoolParameter,
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
            Self::Clip | Self::Blend => "Pose",
            Self::State | Self::Transition => "State Machine",
            Self::FloatParameter | Self::BoolParameter => "Parameters",
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
            Self::State => {
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
    fn label(self) -> &'static str {
        match self {
            Self::Clip => "Animation Clip",
            Self::Blend => "Blend",
            Self::State => "State",
            Self::Transition => "Transition",
            Self::FloatParameter => "Float Parameter",
            Self::BoolParameter => "Bool Parameter",
            Self::Output => "Output",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Clip => "Samples a Bevy animation clip.",
            Self::Blend => "Interpolates two poses by weight.",
            Self::State => "Names a pose-producing state.",
            Self::Transition => "Chooses between poses with a condition.",
            Self::FloatParameter => "Reads a numeric graph parameter.",
            Self::BoolParameter => "Reads a boolean graph parameter.",
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
            AnimNodeTemplate::State,
            AnimNodeTemplate::Transition,
            AnimNodeTemplate::FloatParameter,
            AnimNodeTemplate::BoolParameter,
            AnimNodeTemplate::Output,
        ]
    }
}

fn sample_graph() -> EditorState {
    let mut editor = EditorState::new(1.0);
    let mut user_state = AnimGraphUiState;

    let idle = add_node(
        &mut editor,
        &mut user_state,
        AnimNodeTemplate::Clip,
        "Idle Clip",
        egui::pos2(80.0, 100.0),
    );
    set_node_input_value(
        &mut editor,
        idle,
        "Clip",
        AnimValue::Text("Animation 0".to_string()),
    );
    let walk = add_node(
        &mut editor,
        &mut user_state,
        AnimNodeTemplate::Clip,
        "Walk Clip",
        egui::pos2(80.0, 320.0),
    );
    set_node_input_value(
        &mut editor,
        walk,
        "Clip",
        AnimValue::Text("Animation 1".to_string()),
    );
    let speed = add_node(
        &mut editor,
        &mut user_state,
        AnimNodeTemplate::FloatParameter,
        "Speed",
        egui::pos2(90.0, 560.0),
    );
    let blend = add_node(
        &mut editor,
        &mut user_state,
        AnimNodeTemplate::Blend,
        "Locomotion Blend",
        egui::pos2(420.0, 220.0),
    );
    let output = add_node(
        &mut editor,
        &mut user_state,
        AnimNodeTemplate::Output,
        "Preview Output",
        egui::pos2(760.0, 300.0),
    );

    connect(&mut editor, idle, "Pose", blend, "A");
    connect(&mut editor, walk, "Pose", blend, "B");
    connect(&mut editor, speed, "Value", blend, "Weight");
    connect(&mut editor, blend, "Pose", output, "Pose");

    editor
}

fn add_node(
    editor: &mut EditorState,
    user_state: &mut AnimGraphUiState,
    template: AnimNodeTemplate,
    label: &str,
    position: egui::Pos2,
) -> NodeId {
    let node = editor.graph.add_node(
        label.to_string(),
        template.user_data(user_state),
        |graph, node_id| template.build_node(graph, user_state, node_id),
    );
    editor.node_positions.insert(node, position);
    editor
        .node_orientations
        .insert(node, egui_graph_edit::NodeOrientation::LeftToRight);
    editor.node_order.push(node);
    node
}

fn connect(
    editor: &mut EditorState,
    output_node: NodeId,
    output: &str,
    input_node: NodeId,
    input: &str,
) {
    let output = editor.graph.nodes[output_node]
        .get_output(output)
        .expect("sample output should exist");
    let input = editor.graph.nodes[input_node]
        .get_input(input)
        .expect("sample input should exist");
    editor.graph.add_connection(output, input);
}

fn set_node_input_value(
    editor: &mut EditorState,
    node: NodeId,
    input_name: &str,
    value: AnimValue,
) {
    let input = editor.graph.nodes[node]
        .get_input(input_name)
        .expect("sample input should exist");
    editor.graph.inputs[input].value = value;
}
