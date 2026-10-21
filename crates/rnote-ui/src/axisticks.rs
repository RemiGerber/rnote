// Imports
use crate::{RnAppWindow, RnCanvas};
use gettextrs::gettext;
use gtk4::{
    Align, Box, Button, Orientation, Popover, PositionType, Scale, gdk, glib, glib::clone,
    prelude::*,
};
use rnote_engine::strokes::AxisTicks;

const POPOVER_NAME: &str = "axis-ticks-popover";

fn find_popover(canvas: &RnCanvas) -> Option<Popover> {
    let mut child = canvas.first_child();
    while let Some(widget) = child {
        if widget.widget_name() == POPOVER_NAME
            && let Ok(popover) = widget.clone().downcast::<Popover>()
        {
            return Some(popover);
        }
        child = widget.next_sibling();
    }
    None
}

/// Hide the popup for the number of unit markings.
pub(crate) fn hide_axis_ticks_popover(canvas: &RnCanvas) {
    if let Some(popover) = find_popover(canvas) {
        popover.popdown();
    }
}

/// Show a popup next to the coordinate system that was just drawn, to set the number of unit markings
/// on each half-axis, with plus and minus buttons, a slider and the keyboard.
pub(crate) fn show_axis_ticks_popover(appwindow: &RnAppWindow, canvas: &RnCanvas) {
    let Some(edit) = canvas.engine_ref().axis_ticks_edit() else {
        return;
    };

    // The anchor is in document coordinates
    let (mins, maxs) = (edit.anchor.mins, edit.anchor.maxs);
    let transform = canvas.engine_ref().camera.transform();
    let (top_left, bottom_right) = (
        transform.transform_point2(mins),
        transform.transform_point2(maxs),
    );
    let rect = gdk::Rectangle::new(
        top_left.x.floor() as i32,
        top_left.y.floor() as i32,
        ((bottom_right.x - top_left.x).ceil() as i32).max(1),
        ((bottom_right.y - top_left.y).ceil() as i32).max(1),
    );

    let minus_button = Button::builder()
        .icon_name("value-decrease-symbolic")
        .tooltip_text(gettext("Fewer ticks (-)"))
        .css_classes(["circular"])
        .build();
    let plus_button = Button::builder()
        .icon_name("value-increase-symbolic")
        .tooltip_text(gettext("More ticks (+)"))
        .css_classes(["circular"])
        .build();
    let scale = Scale::with_range(
        Orientation::Horizontal,
        AxisTicks::COUNT_MIN as f64,
        AxisTicks::COUNT_MAX as f64,
        1.0,
    );
    scale.set_digits(0);
    scale.set_draw_value(true);
    scale.set_value_pos(PositionType::Top);
    scale.set_round_digits(0);
    scale.set_width_request(220);
    scale.set_hexpand(true);
    scale.set_value(edit.count as f64);
    scale.set_tooltip_text(Some(&if edit.constant_spacing {
        gettext("Number of ticks on the shortest half-axis")
    } else {
        gettext("Number of ticks on each half-axis")
    }));

    let content = Box::builder()
        .orientation(Orientation::Horizontal)
        .spacing(9)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .valign(Align::Center)
        .build();
    content.append(&minus_button);
    content.append(&scale);
    content.append(&plus_button);

    let done_button = Button::builder()
        .label(gettext("Done"))
        .tooltip_text(gettext("Close (Enter)"))
        .valign(Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();
    content.append(&done_button);

    let popover = Popover::builder()
        .name(POPOVER_NAME)
        .child(&content)
        .position(PositionType::Bottom)
        .pointing_to(&rect)
        .has_arrow(true)
        .autohide(true)
        .build();
    popover.set_parent(canvas);

    scale.connect_value_changed(clone!(
        #[weak]
        appwindow,
        #[weak]
        canvas,
        move |scale| {
            let widget_flags = canvas
                .engine_mut()
                .axis_ticks_set_count(scale.value().round() as u32);
            appwindow.handle_widget_flags(widget_flags, &canvas);
        }
    ));
    minus_button.connect_clicked(clone!(
        #[weak]
        scale,
        move |_| scale.set_value(scale.value() - 1.0)
    ));
    plus_button.connect_clicked(clone!(
        #[weak]
        scale,
        move |_| scale.set_value(scale.value() + 1.0)
    ));

    done_button.connect_clicked(clone!(
        #[weak]
        popover,
        move |_| popover.popdown()
    ));

    // Keyboard: plus and minus change the count, enter confirms
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key_controller.connect_key_pressed(clone!(
        #[weak]
        scale,
        #[weak]
        popover,
        #[upgrade_or]
        glib::Propagation::Proceed,
        move |_, key, _, _| match key {
            gdk::Key::plus
            | gdk::Key::KP_Add
            | gdk::Key::equal
            | gdk::Key::Up
            | gdk::Key::Right => {
                scale.set_value(scale.value() + 1.0);
                glib::Propagation::Stop
            }
            gdk::Key::minus
            | gdk::Key::KP_Subtract
            | gdk::Key::underscore
            | gdk::Key::Down
            | gdk::Key::Left => {
                scale.set_value(scale.value() - 1.0);
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                popover.popdown();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    ));
    popover.add_controller(key_controller);

    popover.connect_closed(clone!(
        #[weak]
        appwindow,
        #[weak]
        canvas,
        move |popover| {
            let widget_flags = canvas.engine_mut().axis_ticks_finish();
            appwindow.handle_widget_flags(widget_flags, &canvas);
            // Unparenting inside the signal handler can crash GTK, so defer it
            let popover = popover.clone();
            glib::idle_add_local_once(move || popover.unparent());
        }
    ));

    popover.popup();
    scale.grab_focus();
}
