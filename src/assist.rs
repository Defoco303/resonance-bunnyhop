use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssistPhase {
    Idle,
    JumpPressed,
    Neutralizing,
    RestoringMovement,
}

impl Default for AssistPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JumpButton {
    Circle,
    Cross,
    Square,
    Triangle,
    L1,
    R1,
    L2Button,
    R2Button,
    L3,
    R3,
    Create,
    Options,
    Touchpad,
    Ps,
}

impl Default for JumpButton {
    fn default() -> Self {
        Self::Circle
    }
}

impl JumpButton {
    pub const ALL: [Self; 14] = [
        Self::Circle,
        Self::Cross,
        Self::Square,
        Self::Triangle,
        Self::L1,
        Self::R1,
        Self::L2Button,
        Self::R2Button,
        Self::L3,
        Self::R3,
        Self::Create,
        Self::Options,
        Self::Touchpad,
        Self::Ps,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Circle => "Circle",
            Self::Cross => "Cross",
            Self::Square => "Square",
            Self::Triangle => "Triangle",
            Self::L1 => "L1",
            Self::R1 => "R1",
            Self::L2Button => "L2 click",
            Self::R2Button => "R2 click",
            Self::L3 => "L3",
            Self::R3 => "R3",
            Self::Create => "Create",
            Self::Options => "Options",
            Self::Touchpad => "Touchpad",
            Self::Ps => "PS",
        }
    }

    pub fn capture_mask(self) -> u32 {
        1_u32 << (self as u32)
    }

    pub fn from_capture_mask(mask: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|button| mask & button.capture_mask() != 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DpadDirection {
    Neutral,
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl Default for DpadDirection {
    fn default() -> Self {
        Self::Neutral
    }
}

impl DpadDirection {
    pub fn from_hat(value: u8) -> Self {
        match value & 0x0f {
            0x0 => Self::North,
            0x1 => Self::NorthEast,
            0x2 => Self::East,
            0x3 => Self::SouthEast,
            0x4 => Self::South,
            0x5 => Self::SouthWest,
            0x6 => Self::West,
            0x7 => Self::NorthWest,
            _ => Self::Neutral,
        }
    }

    pub fn up(self) -> bool {
        matches!(self, Self::North | Self::NorthEast | Self::NorthWest)
    }

    pub fn down(self) -> bool {
        matches!(self, Self::South | Self::SouthEast | Self::SouthWest)
    }

    pub fn left(self) -> bool {
        matches!(self, Self::West | Self::NorthWest | Self::SouthWest)
    }

    pub fn right(self) -> bool {
        matches!(self, Self::East | Self::NorthEast | Self::SouthEast)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Buttons {
    pub square: bool,
    pub circle: bool,
    pub cross: bool,
    pub triangle: bool,
    pub l1: bool,
    pub r1: bool,
    pub l2_button: bool,
    pub r2_button: bool,
    pub create: bool,
    pub options: bool,
    pub l3: bool,
    pub r3: bool,
    pub ps: bool,
    pub touchpad: bool,
    pub mute: bool,
}

impl Buttons {
    pub fn pressed(self, jump_button: JumpButton) -> bool {
        match jump_button {
            JumpButton::Circle => self.circle,
            JumpButton::Cross => self.cross,
            JumpButton::Square => self.square,
            JumpButton::Triangle => self.triangle,
            JumpButton::L1 => self.l1,
            JumpButton::R1 => self.r1,
            JumpButton::L2Button => self.l2_button,
            JumpButton::R2Button => self.r2_button,
            JumpButton::L3 => self.l3,
            JumpButton::R3 => self.r3,
            JumpButton::Create => self.create,
            JumpButton::Options => self.options,
            JumpButton::Touchpad => self.touchpad,
            JumpButton::Ps => self.ps,
        }
    }

    pub fn capture_mask(self) -> u32 {
        let mut mask = 0_u32;
        for button in JumpButton::ALL {
            if self.pressed(button) {
                mask |= button.capture_mask();
            }
        }
        mask
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StickState {
    pub raw_x: u8,
    pub raw_y: u8,
}

impl StickState {
    pub const fn new(raw_x: u8, raw_y: u8) -> Self {
        Self { raw_x, raw_y }
    }

    pub const fn neutral() -> Self {
        Self::new(128, 128)
    }

    pub fn xinput_x(self) -> i16 {
        axis_from_raw(self.raw_x)
    }

    pub fn xinput_y(self) -> i16 {
        let value = -(axis_from_raw(self.raw_y) as i32);
        value.clamp(i16::MIN as i32, i16::MAX as i32) as i16
    }

    pub fn normalized_x(self) -> f32 {
        normalize_axis(self.raw_x)
    }

    pub fn normalized_y(self) -> f32 {
        -normalize_axis(self.raw_y)
    }

    pub fn magnitude(self) -> f32 {
        let x = self.normalized_x();
        let y = self.normalized_y();
        (x * x + y * y).sqrt()
    }

    pub fn is_moving(self, deadzone: f32) -> bool {
        self.magnitude() >= deadzone
    }
}

fn axis_from_raw(value: u8) -> i16 {
    if value >= 128 {
        let delta = value as i32 - 128;
        ((delta * 32_767) / 127) as i16
    } else {
        let delta = value as i32 - 128;
        ((delta * 32_768) / 128) as i16
    }
}

fn normalize_axis(value: u8) -> f32 {
    if value >= 128 {
        (value as f32 - 128.0) / 127.0
    } else {
        (value as f32 - 128.0) / 128.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ControllerState {
    pub left_stick: StickState,
    pub right_stick: StickState,
    pub l2: u8,
    pub r2: u8,
    pub dpad: DpadDirection,
    pub buttons: Buttons,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AssistConfig {
    pub jump_button: JumpButton,
    pub jump_overlap_ms: u64,
    pub movement_release_ms: u64,
    pub retrigger_guard_ms: u64,
    pub movement_deadzone: f32,
    pub only_assist_while_moving: bool,
}

impl Default for AssistConfig {
    fn default() -> Self {
        Self {
            jump_button: JumpButton::Circle,
            jump_overlap_ms: 8,
            movement_release_ms: 20,
            retrigger_guard_ms: 8,
            movement_deadzone: 0.20,
            only_assist_while_moving: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct JumpSequence {
    started_at: Instant,
    movement_hold_until: Instant,
    neutral_until: Instant,
    finished_at: Instant,
}

impl JumpSequence {
    fn new(started_at: Instant, config: AssistConfig) -> Self {
        let movement_hold_until = started_at + Duration::from_millis(config.jump_overlap_ms);
        let neutral_until = movement_hold_until + Duration::from_millis(config.movement_release_ms);
        let finished_at = neutral_until + Duration::from_millis(config.retrigger_guard_ms);
        Self {
            started_at,
            movement_hold_until,
            neutral_until,
            finished_at,
        }
    }

    fn phase(self, now: Instant) -> AssistPhase {
        if now < self.movement_hold_until {
            AssistPhase::JumpPressed
        } else if now < self.neutral_until {
            AssistPhase::Neutralizing
        } else if now < self.finished_at {
            AssistPhase::RestoringMovement
        } else {
            AssistPhase::Idle
        }
    }

    fn movement_suppressed(self, now: Instant) -> bool {
        now >= self.movement_hold_until && now < self.neutral_until
    }

    fn is_finished(self, now: Instant) -> bool {
        now >= self.finished_at
    }

    fn age_ms(self, now: Instant) -> u128 {
        now.duration_since(self.started_at).as_millis()
    }
}

#[derive(Debug, Default)]
pub struct AssistEngine {
    sequence: Option<JumpSequence>,
    previous_jump_pressed: bool,
    phase: AssistPhase,
    jump_count: u64,
    last_sequence_age_ms: u128,
}

impl AssistEngine {
    pub fn phase(&self) -> AssistPhase {
        self.phase
    }

    pub fn has_pending_sequence(&self) -> bool {
        self.sequence.is_some()
    }

    pub fn jump_count(&self) -> u64 {
        self.jump_count
    }

    pub fn last_sequence_age_ms(&self) -> u128 {
        self.last_sequence_age_ms
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn apply(
        &mut self,
        input: ControllerState,
        config: AssistConfig,
        now: Instant,
    ) -> ControllerState {
        let jump_pressed = input.buttons.pressed(config.jump_button);
        let movement_active = input.left_stick.is_moving(config.movement_deadzone);
        let assist_allowed = !config.only_assist_while_moving || movement_active;
        let rising_edge = jump_pressed && !self.previous_jump_pressed;

        if self.sequence.is_some_and(|sequence| sequence.is_finished(now)) {
            self.sequence = None;
        }

        if rising_edge && assist_allowed && self.sequence.is_none() {
            self.sequence = Some(JumpSequence::new(now, config));
            self.jump_count = self.jump_count.saturating_add(1);
        }

        let mut output = input;

        if let Some(sequence) = self.sequence {
            self.phase = sequence.phase(now);
            self.last_sequence_age_ms = sequence.age_ms(now);

            if sequence.movement_suppressed(now) {
                output.left_stick = StickState::neutral();
            }
        } else {
            self.phase = AssistPhase::Idle;
            self.last_sequence_age_ms = 0;
        }

        self.previous_jump_pressed = jump_pressed;
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stick_conversion_matches_expected_extremes() {
        assert_eq!(StickState::new(128, 128).xinput_x(), 0);
        assert_eq!(StickState::new(128, 128).xinput_y(), 0);
        assert_eq!(StickState::new(0, 128).xinput_x(), i16::MIN);
        assert_eq!(StickState::new(255, 128).xinput_x(), i16::MAX);
        assert_eq!(StickState::new(128, 0).xinput_y(), i16::MAX);
        assert_eq!(StickState::new(128, 255).xinput_y(), -32_767);
    }

    #[test]
    fn assist_sequence_neutralizes_then_presses_jump() {
        let config = AssistConfig {
            jump_button: JumpButton::Circle,
            jump_overlap_ms: 10,
            movement_release_ms: 20,
            retrigger_guard_ms: 10,
            movement_deadzone: 0.1,
            only_assist_while_moving: true,
        };
        let base = Instant::now();
        let mut engine = AssistEngine::default();
        let moving_jump = ControllerState {
            left_stick: StickState::new(255, 128),
            buttons: Buttons {
                circle: true,
                ..Buttons::default()
            },
            ..ControllerState::default()
        };

        let overlapped = engine.apply(moving_jump, config, base);
        assert_eq!(overlapped.left_stick, moving_jump.left_stick);
        assert!(overlapped.buttons.circle);
        assert_eq!(engine.phase(), AssistPhase::JumpPressed);

        let neutralized = engine.apply(moving_jump, config, base + Duration::from_millis(12));
        assert_eq!(neutralized.left_stick, StickState::neutral());
        assert!(neutralized.buttons.circle);
        assert_eq!(engine.phase(), AssistPhase::Neutralizing);

        let restored = engine.apply(moving_jump, config, base + Duration::from_millis(35));
        assert_eq!(restored.left_stick, moving_jump.left_stick);
        assert!(restored.buttons.circle);
        assert_eq!(engine.phase(), AssistPhase::RestoringMovement);

        let idle = engine.apply(
            ControllerState {
                buttons: Buttons::default(),
                ..moving_jump
            },
            config,
            base + Duration::from_millis(45),
        );
        assert_eq!(idle.left_stick, moving_jump.left_stick);
        assert!(!idle.buttons.circle);
        assert_eq!(engine.phase(), AssistPhase::Idle);
    }
}
