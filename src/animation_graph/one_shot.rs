use egui_graph_edit::{Graph, NodeId};

use super::{AnimDataType, AnimNodeData, AnimValue};

pub fn one_shot_lane_count(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> usize {
    one_shot_lanes(graph, node).len().max(1)
}

pub fn one_shot_lanes(
    graph: &Graph<AnimNodeData, AnimDataType, AnimValue>,
    node: NodeId,
) -> Vec<usize> {
    let Some(node) = graph.nodes.get(node) else {
        return vec![1];
    };

    let mut lanes: Vec<_> = node
        .inputs
        .iter()
        .filter_map(|(name, _)| one_shot_action_lane(name))
        .collect();
    lanes.sort_unstable();
    lanes.dedup();
    if lanes.is_empty() {
        lanes.push(1);
    }
    lanes
}

pub fn one_shot_action_input_name(lane: usize) -> String {
    if lane <= 1 {
        "Action".to_string()
    } else {
        format!("Action {lane}")
    }
}

pub fn one_shot_trigger_input_name(lane: usize) -> String {
    if lane <= 1 {
        "Trigger".to_string()
    } else {
        format!("Trigger {lane}")
    }
}

pub fn one_shot_fade_in_input_name(lane: usize) -> String {
    if lane <= 1 {
        "Fade In".to_string()
    } else {
        format!("Fade In {lane}")
    }
}

pub fn one_shot_fade_out_input_name(lane: usize) -> String {
    if lane <= 1 {
        "Fade Out".to_string()
    } else {
        format!("Fade Out {lane}")
    }
}

pub fn one_shot_playback_input_name(lane: usize) -> String {
    if lane <= 1 {
        "Playback".to_string()
    } else {
        format!("Playback {lane}")
    }
}

pub fn one_shot_speed_input_name(lane: usize) -> String {
    if lane <= 1 {
        "Speed".to_string()
    } else {
        format!("Speed {lane}")
    }
}

pub fn one_shot_start_offset_input_name(lane: usize) -> String {
    if lane <= 1 {
        "Start Offset".to_string()
    } else {
        format!("Start Offset {lane}")
    }
}

pub(super) fn one_shot_action_lane(name: &str) -> Option<usize> {
    if name == "Action" {
        return Some(1);
    }
    name.strip_prefix("Action ")
        .and_then(|suffix| suffix.parse::<usize>().ok())
        .filter(|lane| *lane > 1)
}

pub(super) fn one_shot_trigger_lane(name: &str) -> Option<usize> {
    if name == "Trigger" {
        return Some(1);
    }
    name.strip_prefix("Trigger ")
        .and_then(|suffix| suffix.parse::<usize>().ok())
        .filter(|lane| *lane > 1)
}

pub(super) fn one_shot_start_offset_lane(name: &str) -> Option<usize> {
    if name == "Start Offset" {
        return Some(1);
    }
    name.strip_prefix("Start Offset ")
        .and_then(|suffix| suffix.parse::<usize>().ok())
        .filter(|lane| *lane > 1)
}

pub(super) fn is_one_shot_fade_input(name: &str) -> bool {
    name == "Fade In"
        || name == "Fade Out"
        || name.strip_prefix("Fade In ").is_some_and(is_lane_suffix)
        || name.strip_prefix("Fade Out ").is_some_and(is_lane_suffix)
}

pub(super) fn is_one_shot_playback_input(name: &str) -> bool {
    name == "Playback" || name.strip_prefix("Playback ").is_some_and(is_lane_suffix)
}

pub(super) fn is_one_shot_start_offset_input(name: &str) -> bool {
    name == "Start Offset"
        || name
            .strip_prefix("Start Offset ")
            .is_some_and(is_lane_suffix)
}

fn is_lane_suffix(value: &str) -> bool {
    value.parse::<usize>().is_ok_and(|lane| lane > 1)
}
