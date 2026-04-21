use vigem_rust::{X360Button, X360Report};

use crate::assist::ControllerState;

pub fn map_to_x360_report(state: ControllerState) -> X360Report {
    let mut buttons = X360Button::empty();

    buttons.set(X360Button::A, state.buttons.cross);
    buttons.set(X360Button::B, state.buttons.circle);
    buttons.set(X360Button::X, state.buttons.square);
    buttons.set(X360Button::Y, state.buttons.triangle);
    buttons.set(X360Button::LEFT_SHOULDER, state.buttons.l1);
    buttons.set(X360Button::RIGHT_SHOULDER, state.buttons.r1);
    buttons.set(X360Button::BACK, state.buttons.create);
    buttons.set(X360Button::START, state.buttons.options);
    buttons.set(X360Button::LEFT_THUMB, state.buttons.l3);
    buttons.set(X360Button::RIGHT_THUMB, state.buttons.r3);
    buttons.set(X360Button::GUIDE, state.buttons.ps);
    buttons.set(X360Button::DPAD_UP, state.dpad.up());
    buttons.set(X360Button::DPAD_DOWN, state.dpad.down());
    buttons.set(X360Button::DPAD_LEFT, state.dpad.left());
    buttons.set(X360Button::DPAD_RIGHT, state.dpad.right());

    let mut report = X360Report::default();
    report.buttons = buttons;
    report.left_trigger = state.l2;
    report.right_trigger = state.r2;
    report.thumb_lx = state.left_stick.xinput_x();
    report.thumb_ly = state.left_stick.xinput_y();
    report.thumb_rx = state.right_stick.xinput_x();
    report.thumb_ry = state.right_stick.xinput_y();
    report
}
