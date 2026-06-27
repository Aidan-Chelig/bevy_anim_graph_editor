use std::time::Duration;

use bevy_anim_graph_editor::animation_graph::{
    AnimGraphEditor, EdgeVisualization, connection_contribution_value, connection_marker_value,
};
use bevy_egui::egui;
use egui_graph_edit::ConnectionRenderInfo;

pub(crate) fn draw_connection_visualization(
    ui: &egui::Ui,
    editor: &mut AnimGraphEditor,
    connections: &[ConnectionRenderInfo],
    elapsed_secs: f32,
) {
    if matches!(editor.ui_state.edge_visualization, EdgeVisualization::Flow) {
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f32(1.0 / 30.0));
    }

    let delta_secs = editor
        .ui_state
        .flow_last_time
        .map(|last_time| (elapsed_secs - last_time).clamp(0.0, 0.1))
        .unwrap_or(0.0);
    editor.ui_state.flow_last_time = Some(elapsed_secs);

    let painter = ui.painter();
    for connection in connections {
        if editor.ui_state.contribution_borders {
            draw_connection_contribution_border(
                painter,
                connection,
                connection_contribution_value(
                    &editor.graph.graph,
                    connection.input,
                    editor.preview_output,
                    &editor.ui_state.live_one_shot_progress,
                ),
            );
        }

        let Some(value) = connection_marker_value(
            &editor.graph.graph,
            connection.input,
            connection.output,
            &editor.ui_state.live_one_shot_progress,
        ) else {
            continue;
        };

        match editor.ui_state.edge_visualization {
            EdgeVisualization::Marker => draw_connection_marker(painter, connection.points, value),
            EdgeVisualization::Flow => {
                let phase = editor
                    .ui_state
                    .flow_phases
                    .entry((connection.input, connection.output))
                    .or_insert(0.0);
                draw_connection_flow(painter, connection.points, value, phase, delta_secs);
            }
        }
    }
}

fn draw_connection_contribution_border(
    painter: &egui::Painter,
    connection: &ConnectionRenderInfo,
    contribution: f32,
) {
    let contribution = contribution.clamp(0.0, 1.0);
    if contribution <= 0.001 {
        return;
    }

    let alpha = (35.0 + contribution * 185.0) as u8;
    let glow = egui::epaint::CubicBezierShape::from_points_stroke(
        connection.points,
        false,
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(
            12.0,
            egui::Color32::from_rgba_unmultiplied(126, 236, 255, alpha),
        ),
    );
    painter.add(glow);

    let foreground = egui::epaint::CubicBezierShape::from_points_stroke(
        connection.points,
        false,
        egui::Color32::TRANSPARENT,
        egui::Stroke::new(5.0, connection.color),
    );
    painter.add(foreground);
}

fn draw_connection_marker(painter: &egui::Painter, points: [egui::Pos2; 4], value: f32) {
    let position = cubic_bezier_point(points, value);
    painter.circle_filled(position, 5.0, egui::Color32::from_rgb(245, 176, 75));
    painter.circle_stroke(
        position,
        6.0,
        egui::Stroke::new(1.25, egui::Color32::from_rgb(25, 28, 31)),
    );
}

fn draw_connection_flow(
    painter: &egui::Painter,
    points: [egui::Pos2; 4],
    value: f32,
    phase: &mut f32,
    delta_secs: f32,
) {
    let value = value.clamp(0.0, 1.0);

    const FLOW_DOT_SLOTS: usize = 10;

    let curve = SampledBezier::new(points);
    if curve.length <= f32::EPSILON {
        return;
    }
    let speed_pixels_per_second = 18.0 + value * 95.0;
    *phase = (*phase + delta_secs * speed_pixels_per_second / curve.length).fract();

    let alpha = (35.0 + value * 220.0) as u8;
    if alpha == 0 {
        return;
    }
    let radius = 2.5 + value * 2.0;
    let color = egui::Color32::from_rgba_unmultiplied(245, 176, 75, alpha);
    let outline = egui::Color32::from_rgba_unmultiplied(25, 28, 31, alpha.saturating_sub(25));

    for index in 0..FLOW_DOT_SLOTS {
        let spacing = index as f32 / FLOW_DOT_SLOTS as f32;
        let distance_fraction = (*phase + spacing).fract();
        let position = curve.point_at_distance_fraction(distance_fraction);
        painter.circle_filled(position, radius, color);
        painter.circle_stroke(position, radius + 1.0, egui::Stroke::new(1.0, outline));
    }
}

struct SampledBezier {
    points: [egui::Pos2; 33],
    cumulative_lengths: [f32; 33],
    length: f32,
}

impl SampledBezier {
    fn new(control_points: [egui::Pos2; 4]) -> Self {
        let mut points = [control_points[0]; 33];
        let mut cumulative_lengths = [0.0; 33];

        // Flow dots move by approximate arc length instead of raw Bezier `t`; otherwise they
        // visibly speed up and slow down depending on the curve shape.
        for index in 1..=32 {
            let t = index as f32 / 32.0;
            points[index] = cubic_bezier_point(control_points, t);
            cumulative_lengths[index] =
                cumulative_lengths[index - 1] + points[index - 1].distance(points[index]);
        }

        Self {
            points,
            cumulative_lengths,
            length: cumulative_lengths[32],
        }
    }

    fn point_at_distance_fraction(&self, distance_fraction: f32) -> egui::Pos2 {
        let target = self.length * distance_fraction.clamp(0.0, 1.0);
        for index in 1..=32 {
            if self.cumulative_lengths[index] >= target {
                let segment_start = self.cumulative_lengths[index - 1];
                let segment_length = self.cumulative_lengths[index] - segment_start;
                let local_t = if segment_length <= f32::EPSILON {
                    0.0
                } else {
                    (target - segment_start) / segment_length
                };
                return self.points[index - 1].lerp(self.points[index], local_t);
            }
        }

        self.points[32]
    }
}

fn cubic_bezier_point(points: [egui::Pos2; 4], t: f32) -> egui::Pos2 {
    let t = t.clamp(0.0, 1.0);
    let inverse = 1.0 - t;
    let [p0, p1, p2, p3] = points;
    let point = p0.to_vec2() * inverse.powi(3)
        + p1.to_vec2() * 3.0 * inverse.powi(2) * t
        + p2.to_vec2() * 3.0 * inverse * t.powi(2)
        + p3.to_vec2() * t.powi(3);
    point.to_pos2()
}
