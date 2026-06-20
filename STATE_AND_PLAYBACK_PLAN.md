# State Handling and Playback Plan

This document sketches the next runtime/editor shape for animation state machines, transitions, and clip playback behavior.

## Goals

- Keep the visual editor understandable for authoring blends, parameters, and state machines.
- Export enough data for a library/runtime consumer to evaluate the same graph outside the editor.
- Support common playback behavior: looping, one-shot, ping-pong, hold-last-frame, and transitions after clip completion.
- Keep Bevy-native graph export available, while treating richer state logic as editor/runtime metadata when Bevy's native graph does not directly encode it.

## Core Concepts

### Pose Graph

The current node graph remains the pose evaluation graph. It answers: "Given the current parameters and state, what pose should be sampled this frame?"

Existing useful nodes:

- `Clip`: samples a GLB animation clip.
- `Blend`: blends two poses with one weight.
- `WeightedBlend`: blends two poses by normalized weights.
- `Transition`: blends between two poses over time.
- `State`: names and wraps a pose subtree.
- `Output`: marks the preview/runtime output pose.

### State Machine

A state machine should be a higher-level controller that chooses which state pose is active, when to transition, and what happens after playback events.

Proposed model:

```text
StateMachine
  states: StateId -> StateDefinition
  transitions: Vec<StateTransition>
  initial_state: StateId
  active_state: StateId
  active_transition: Option<RunningTransition>
```

Each state points to a pose-producing node:

```text
StateDefinition
  name: String
  pose_node: NodeId
  playback: PlaybackSettings
  on_complete: CompletionAction
```

Each transition owns a condition and blend duration:

```text
StateTransition
  from: StateId
  to: StateId
  condition: ConditionExpression
  duration_seconds: f32
  interrupt: InterruptPolicy
```

## Playback Types

Clip playback should be explicit instead of assuming every clip is a regular loop.

```text
PlaybackSettings
  mode: PlaybackMode
  speed: f32
  start_offset_seconds: f32
```

Suggested modes:

- `Loop`: repeats normally.
- `Once`: plays once, then reports completion.
- `OnceHold`: plays once and holds the final frame.
- `PingPong`: alternates forward and backward indefinitely.
- `PingPongOnce`: plays forward then backward once, then reports completion.
- `Manual`: time is driven by an input parameter, useful for aim offsets, scrubbing, or authored sync.

Completion actions:

- `Stay`: remain in the state.
- `TransitionTo(StateId)`: automatically begin a transition when playback completes.
- `SetParameter { name, value }`: useful for one-shot events that should clear their own trigger.
- `EmitEvent(String)`: lets a game runtime react without hard-coding editor behavior.

## Transition Behavior

Transitions should evaluate in priority order. On each runtime tick:

1. Update parameters from the host app.
2. Advance active state's playback clock.
3. If a transition is active, advance transition progress and blend from source pose to target pose.
4. If no transition is active, evaluate outgoing transitions for the current state.
5. If the active state's playback completed, apply its `CompletionAction`.
6. Evaluate the pose graph using the chosen active state and transition weights.

Interrupt policies:

- `None`: active transition cannot be interrupted.
- `HigherPriority`: only earlier transitions can interrupt.
- `Any`: any valid outgoing transition can interrupt.

Completion transitions should be lower priority than explicit gameplay transitions unless authored otherwise.

## Editor UX

Short-term editor additions:

- Add playback settings to `Clip` and/or `State` nodes.
- Add an `On Complete` field to `State` nodes.
- Flesh out transition nodes with `From`, `To`, `Condition`, `Duration`, and `Interrupt Policy`.
- Add a state-machine visual mode that shows states as nodes and transitions as directed edges.
- Show live state machine diagnostics in preview: active state, active transition, transition progress, clip time, and completion status.

Authoring options:

- Simple users can keep using the current pose graph directly.
- State-machine users can create states that reference pose subtrees.
- Advanced users can mix blend trees inside state poses, then use state transitions to move between those blend trees.

## Runtime Export Shape

The runtime export should include both the essential pose graph and optional state machine metadata.

```text
RuntimeAnimGraph
  version: u32
  gltf_asset_path: Option<String>
  preview_output: Option<usize>
  nodes: Vec<RuntimeAnimNode>
  connections: Vec<RuntimeAnimConnection>
  state_machines: Vec<RuntimeStateMachine>
```

The editor project file can keep UI details like node positions, zoom, selection, and preview settings. Runtime export should avoid editor-only state.

## Bevy Export Notes

Bevy's native `AnimationGraph` is the direct pose graph export target. It is useful for clips and weighted blends.

State-machine behavior, playback modes, one-shot completion, ping-pong timing, and transition policies may need a small runtime library layer around Bevy's animation player/graph. The export can still generate a Bevy animation graph for the pose tree, while the runtime metadata drives weights, playback clocks, transition progress, and events.

## Implementation Phases

1. Define serializable runtime types for playback settings, completion actions, transition conditions, and state machines.
2. Add editor fields for playback mode and state completion behavior.
3. Extend preview runtime to track clip time, completion, active state, and active transition.
4. Add state-machine export data to `RuntimeAnimGraph`.
5. Add a native runtime crate API that loads the runtime graph and exposes systems/helpers for Bevy apps.
6. Add validation: missing states, impossible transitions, invalid clip references, non-positive durations, and unsupported completion targets.

