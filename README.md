# Bevy Animation Graph Editor

A native Bevy tool for creating, previewing, saving, loading, and exporting animation graphs. It uses `bevy_egui` and a local patched copy of `egui-graph-edit` for node editing, with a live Bevy 3D preview for GLB/glTF animation clips.

This is currently an editor/runtime prototype. The goal is to make Bevy animation graph authoring easier to inspect visually, while also exporting enough graph data for native Bevy apps or a lightweight runtime library.

## Preview

<video src="README/preview.mp4" controls muted loop></video>

[Open the preview video](README/preview.mp4)

## Features

- Node editor for pose, state, transition, one-shot, parameter, and math nodes.
- Live 3D preview from imported `.glb` or `.gltf` files.
- Project save/load with a graph plus the referenced GLB asset path.
- Missing GLB relinking when loading a project.
- Export to Bevy `AnimationGraph` RON.
- Export to a runtime-oriented graph RON.
- Auto-apply graph edits to the preview.
- Visual edge effects for weights and contribution paths.
- Basic preview camera controls.

## Requirements

- Rust toolchain from `rust-toolchain.toml`.
- Native graphics dependencies required by Bevy.
- Optional but recommended: Nix, using the included `flake.nix`.

On Nix/NixOS:

```bash
nix develop
cargo run
```

Without Nix, install the platform dependencies required by Bevy and run:

```bash
cargo run
```

## Running

Start with an empty editor:

```bash
cargo run
```

Open a GLB/glTF on startup:

```bash
cargo run -- path/to/character.glb
```

Open an editor project on startup:

```bash
cargo run -- assets/graphs/default.animgraph_project.ron
```

## Controls

- `Space`: toggle preview playback.
- `Enter`: cycle raw GLB animations when no editor graph is applied.
- `Shift + left drag`: orbit preview camera.
- `Shift + right drag`: pan preview camera.
- `Shift + scroll`: zoom preview camera.

## File Menu

- `Save`: save the current editor project.
- `Save As...`: save the project to a new path.
- `Load...`: load an editor project.
- `Import GLB...`: import a GLB/glTF into `assets/imports`.
- `Export > Bevy Animation Graph...`: export a Bevy animation graph RON.
- `Export > Runtime Graph...`: export the editor/runtime graph RON.

Editor project files store the editable graph and the referenced GLB asset path. If the GLB is missing when a project is loaded, the editor prompts for a replacement.

## Node Types

Pose nodes:

- `Animation Clip`: samples a clip from the loaded GLB.
- `Blend`: blends two poses with a normalized weight.
- `Weighted Blend`: blends two poses using raw child weights.
- `One Shot`: plays one or more action poses over a base pose, then fades back.
- `Output`: selects the pose tree used by the preview.

State machine nodes:

- `State`: names a pose-producing state.
- `Transition`: transitions between two state/pose branches by condition and duration.

Parameter nodes:

- `Float Parameter`
- `Bool Parameter`
- `Trigger Parameter`

Math nodes:

- `Float Transition`: lerps between two floats by weight.
- `Remap 0..1`
- `Add`
- `Multiply`
- `Invert 1-x`
- `Clamp`
- `Smoothstep`
- `Compare`

## One-Shots

The `One Shot` node supports multiple action lanes. Each lane has:

- `Action`
- `Trigger`
- `Fade In`
- `Fade Out`
- `Playback`
- `Speed`
- `Start Offset`

Use the `+` and `-` controls on the node to add or remove lanes. A common setup is:

```text
locomotion pose -> One Shot Base
interact clip   -> One Shot Action
interact trigger parameter -> One Shot Trigger
One Shot -> Output
```

When the trigger fires, the action lane fades in, plays according to its playback settings, then fades out after the clip completes.

## Speed And Weights

Float parameters and math values are not globally clamped. Only weight-like inputs clamp to `0..1`, such as blend weights and `Float Transition` weight.

Clip, state, and one-shot speed inputs can be constants or connected float graphs. This makes it possible to drive animation playback speed from parameters and math nodes.

## Development

Useful commands:

```bash
nix develop -c cargo fmt
nix develop -c cargo check
nix develop -c cargo run
```

The project patches `egui-graph-edit` through:

```toml
[patch.crates-io]
egui-graph-edit = { path = "crates/egui-graph-edit" }
```

Do not commit imported GLB assets or generated target directories unless they are intentionally part of a test fixture.

## Current Limitations

- The runtime/export format is still evolving.
- State machine behavior is functional but still experimental.
- The Bevy export is useful for previewing and inspection, but runtime consumers may still need the exported runtime graph for editor-specific logic nodes.
- Large GLBs and dense graphs may need further performance work.
