use bevy::{
    gltf::GltfAssetLabel,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    prelude::*,
};

use super::PreviewState;

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
