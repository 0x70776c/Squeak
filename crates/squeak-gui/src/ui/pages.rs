//! Capability-gated libadwaita preference pages, interactive SVG mouse diagram, and error
//! `AdwStatusPage` builders.

use std::cell::RefCell;
use std::rc::Rc;

#[allow(deprecated)]
use gtk4::ColorChooserWidget;
use gtk4::gdk::{RGBA, Texture};
use gtk4::gdk_pixbuf::PixbufLoader;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, Box as GtkBox, Button, DropDown, FlowBox, GestureClick, Label, Orientation,
    Picture, Scale, SelectionMode, SpinButton, StringList,
};
use libadwaita as adw;
use libadwaita::prelude::*;

use crate::backend::{BackendHandle, DeviceSnapshot, UiCommand};
use crate::i18n::tr;
use crate::state::ErrorViewState;
use crate::svg_diagram::{hit_test_button, render_svg_with_highlight};

/// Builds the Overview page (`AdwPreferencesPage`) showing identity, firmware, battery, and mode.
pub fn build_overview_page(snap: &DeviceSnapshot, backend: &BackendHandle) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(tr("Overview"));
    page.set_icon_name(Some("input-mouse-symbolic"));

    let info_group = adw::PreferencesGroup::new();
    info_group.set_title(&snap.summary.matched_name);
    info_group.set_description(Some(&format!(
        "{} {} · HID++ {}",
        snap.summary.vendor_id,
        snap.summary.product_id,
        snap.summary.protocol_version.as_deref().unwrap_or("2.0+")
    )));

    add_kv_row(&info_group, "Bus", &snap.summary.bus);
    add_kv_row(&info_group, "Path", &snap.summary.path);

    if let Some(status) = &snap.status {
        if let Some(fw) = &status.firmware {
            let fw_text = fw
                .entities
                .iter()
                .map(|e| format!("{}: {}", e.kind, e.version))
                .collect::<Vec<_>>()
                .join(" · ");
            if !fw_text.is_empty() {
                add_kv_row(&info_group, tr("Firmware"), &fw_text);
            }
        }
        if let Some(bat) = &status.battery {
            let pct = bat.percentage.map(|p| format!("{p}%")).unwrap_or_else(|| {
                bat.level_bucket
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_owned())
            });
            add_kv_row(
                &info_group,
                tr("Battery"),
                &format!("{pct} ({})", bat.charging_status),
            );
        }
        if let Some(dpi) = &status.dpi
            && let Some(s0) = dpi.sensors.first()
        {
            add_kv_row(&info_group, "DPI", &format!("{} DPI", s0.current_dpi));
        }
        if let Some(rate) = &status.report_rate {
            add_kv_row(
                &info_group,
                tr("Polling Rate"),
                &format!("{} Hz", rate.current_hz),
            );
        }
    }
    page.add(&info_group);

    if let Some(mode_rep) = &snap.onboard_mode {
        let mode_group = adw::PreferencesGroup::new();
        mode_group.set_title(tr("Operating Mode"));

        let row = adw::ActionRow::new();
        row.set_title(tr("Operating Mode"));
        let is_host = mode_rep.mode == "host";
        row.set_subtitle(if is_host {
            tr("Host (Software)")
        } else {
            tr("Onboard Memory")
        });

        let switch_box = GtkBox::new(Orientation::Horizontal, 6);
        switch_box.set_valign(Align::Center);

        let host_btn = Button::with_label(tr("Host (Software)"));
        let onboard_btn = Button::with_label(tr("Onboard Memory"));
        if is_host {
            host_btn.add_css_class("suggested-action");
        } else {
            onboard_btn.add_css_class("suggested-action");
        }

        let key = snap.summary.device_key.clone();
        let b1 = backend.clone();
        host_btn.connect_clicked(move |_| {
            b1.send(UiCommand::SetOnboardMode {
                device_key: key.clone(),
                mode: "host".to_owned(),
            });
        });

        let key = snap.summary.device_key.clone();
        let b2 = backend.clone();
        onboard_btn.connect_clicked(move |_| {
            b2.send(UiCommand::SetOnboardMode {
                device_key: key.clone(),
                mode: "onboard".to_owned(),
            });
        });

        switch_box.append(&host_btn);
        switch_box.append(&onboard_btn);
        row.add_suffix(&switch_box);
        mode_group.add(&row);
        page.add(&mode_group);
    }

    page
}

/// Builds the DPI & Polling Rate page with debounced live slider, up to 6 DPI presets, and rate selector.
pub fn build_dpi_page(
    snap: &DeviceSnapshot,
    backend: &BackendHandle,
    dpi_presets: &Rc<RefCell<Vec<u16>>>,
) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(tr("DPI & Sensor"));
    page.set_icon_name(Some("preferences-system-symbolic"));

    if let Some(status) = &snap.status {
        if let Some(dpi_cfg) = &status.dpi {
            for sensor in &dpi_cfg.sensors {
                let group = adw::PreferencesGroup::new();
                group.set_title(&format!("Sensor {}", sensor.sensor_index));

                let (min_dpi, max_dpi, step) = if let Some(r) = sensor.capabilities.ranges.first() {
                    (
                        f64::from(r.min),
                        f64::from(r.max),
                        f64::from(r.step.max(50)),
                    )
                } else {
                    (100.0, 25600.0, 50.0)
                };

                let row = adw::ActionRow::new();
                row.set_title("DPI");
                let value_label = Label::new(Some(&format!("{} DPI", sensor.current_dpi)));
                value_label.add_css_class("numeric");
                row.add_suffix(&value_label);

                let adj = Adjustment::new(
                    f64::from(sensor.current_dpi),
                    min_dpi,
                    max_dpi,
                    step,
                    step * 2.0,
                    0.0,
                );
                let scale = Scale::new(Orientation::Horizontal, Some(&adj));
                scale.set_hexpand(true);
                scale.set_draw_value(false);
                scale.set_size_request(220, -1);
                scale.update_property(&[gtk4::accessible::Property::Label("DPI slider")]);

                let suppress_slider = Rc::new(RefCell::new(false));
                let last_sent_dpi = Rc::new(RefCell::new(sensor.current_dpi));
                let debounce_source: Rc<RefCell<Option<glib::SourceId>>> =
                    Rc::new(RefCell::new(None));

                let presets_flow = FlowBox::new();
                presets_flow.set_selection_mode(SelectionMode::None);
                presets_flow.set_max_children_per_line(6);
                presets_flow.set_column_spacing(8);
                presets_flow.set_row_spacing(8);
                presets_flow.set_margin_top(6);
                presets_flow.set_margin_bottom(6);

                let rebuild_ctx = PresetRebuildCtx {
                    flow: presets_flow.clone(),
                    presets: Rc::clone(dpi_presets),
                    device_key: snap.summary.device_key.clone(),
                    sensor_index: sensor.sensor_index,
                    backend: backend.clone(),
                    scale: scale.clone(),
                    value_label: value_label.clone(),
                    suppress_slider: Rc::clone(&suppress_slider),
                    last_sent_dpi: Rc::clone(&last_sent_dpi),
                };
                rebuild_preset_buttons(&rebuild_ctx);

                {
                    let key = snap.summary.device_key.clone();
                    let sensor_idx = sensor.sensor_index;
                    let b = backend.clone();
                    let lbl = value_label.clone();
                    let sup = Rc::clone(&suppress_slider);
                    let last = Rc::clone(&last_sent_dpi);
                    let ctx_for_slider = rebuild_ctx.clone();
                    let ds_outer = Rc::clone(&debounce_source);

                    scale.connect_value_changed(move |s| {
                        if *sup.borrow() {
                            return;
                        }
                        let raw = s.value();
                        let snapped_f = ((raw / step).round() * step).clamp(min_dpi, max_dpi);
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let target_dpi = snapped_f as u16;
                        lbl.set_text(&format!("{target_dpi} DPI"));

                        if let Some(prev) = ds_outer.borrow_mut().take() {
                            prev.remove();
                        }
                        let key_inner = key.clone();
                        let b_inner = b.clone();
                        let ds_clone = Rc::clone(&ds_outer);
                        let last_inner = Rc::clone(&last);
                        let ctx_inner = ctx_for_slider.clone();
                        let id = glib::timeout_add_local_once(
                            std::time::Duration::from_millis(220),
                            move || {
                                *ds_clone.borrow_mut() = None;
                                if *last_inner.borrow() == target_dpi {
                                    return;
                                }
                                *last_inner.borrow_mut() = target_dpi;
                                rebuild_preset_buttons(&ctx_inner);
                                b_inner.send(UiCommand::SetDpi {
                                    device_key: key_inner,
                                    sensor_index: sensor_idx,
                                    dpi: target_dpi,
                                });
                            },
                        );
                        *ds_outer.borrow_mut() = Some(id);
                    });
                }

                let slider_row = adw::ActionRow::new();
                slider_row.set_child(Some(&scale));

                group.add(&row);
                group.add(&slider_row);
                page.add(&group);

                // DPI Presets Group (default 700, 1000, 1200, 3600; up to 6 stages)
                let presets_group = adw::PreferencesGroup::new();
                presets_group.set_title(tr("DPI Presets"));
                presets_group.set_description(Some(tr(
                    "Click a preset to apply immediately, or add custom stages (max 6).",
                )));

                let presets_row = adw::ActionRow::new();
                presets_row.set_child(Some(&presets_flow));
                presets_group.add(&presets_row);

                let add_row = adw::ActionRow::new();
                add_row.set_title(tr("Add Custom DPI"));
                let spin_adj = Adjustment::new(1600.0, min_dpi, max_dpi, step, step * 2.0, 0.0);
                let spin = SpinButton::new(Some(&spin_adj), step, 0);
                spin.set_valign(Align::Center);
                let add_btn = Button::with_label(tr("+ Add"));
                add_btn.add_css_class("suggested-action");
                add_btn.set_valign(Align::Center);

                {
                    let ctx_add = rebuild_ctx.clone();
                    let spin_ref = spin.clone();
                    add_btn.connect_clicked(move |_| {
                        let raw = spin_ref.value();
                        let snapped_f = ((raw / step).round() * step).clamp(min_dpi, max_dpi);
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        let dpi_val = snapped_f as u16;
                        {
                            let mut list = ctx_add.presets.borrow_mut();
                            if list.len() < 6 && !list.contains(&dpi_val) {
                                list.push(dpi_val);
                                list.sort_unstable();
                            }
                        }
                        rebuild_preset_buttons(&ctx_add);
                    });
                }

                let add_box = GtkBox::new(Orientation::Horizontal, 8);
                add_box.set_valign(Align::Center);
                add_box.append(&spin);
                add_box.append(&add_btn);
                add_row.add_suffix(&add_box);
                presets_group.add(&add_row);
                page.add(&presets_group);
            }
        }

        if let Some(rate) = &status.report_rate {
            let rate_group = adw::PreferencesGroup::new();
            rate_group.set_title(tr("Polling Rate"));

            let row = adw::ActionRow::new();
            row.set_title(tr("Polling Rate"));
            row.set_subtitle(&format!("{} Hz", rate.current_hz));

            let btn_box = GtkBox::new(Orientation::Horizontal, 6);
            btn_box.set_valign(Align::Center);
            for &hz in &rate.supported_hz {
                let btn = Button::with_label(&format!("{hz} Hz"));
                if hz == rate.current_hz {
                    btn.add_css_class("suggested-action");
                }
                let key = snap.summary.device_key.clone();
                let b = backend.clone();
                btn.connect_clicked(move |_| {
                    b.send(UiCommand::SetReportRate {
                        device_key: key.clone(),
                        hz,
                    });
                });
                btn_box.append(&btn);
            }
            row.add_suffix(&btn_box);
            rate_group.add(&row);
            page.add(&rate_group);
        }
    }

    page
}

/// Builds the interactive SVG Mouse Diagram + Buttons page.
pub fn build_buttons_page(snap: &DeviceSnapshot) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(tr("Buttons"));
    page.set_icon_name(Some("input-keyboard-symbolic"));

    let Some(buttons_rep) = &snap.buttons else {
        return page;
    };

    let diagram_group = adw::PreferencesGroup::new();
    diagram_group.set_title(&snap.summary.matched_name);
    if !buttons_rep.supports_reprog_v4 && buttons_rep.supports_onboard_profiles {
        diagram_group.set_description(Some(
            "Buttons are configured via Onboard Profiles (0x8100). Click any button on the diagram to inspect.",
        ));
    }

    let svg_name = buttons_rep.svg.clone();
    let picture = Picture::new();
    picture.set_size_request(240, 380);
    picture.set_can_shrink(true);
    picture.set_halign(Align::Center);
    update_svg_picture(&picture, svg_name.as_deref(), None);

    let selected_label = Label::new(Some(tr("Click a button on the mouse diagram")));
    selected_label.set_halign(Align::Center);
    selected_label.add_css_class("dim-label");

    let click = GestureClick::new();
    let pic_for_click = picture.clone();
    let svg_for_click = svg_name.clone();
    let lbl_for_click = selected_label.clone();
    let btns_for_click = buttons_rep.buttons.clone();

    click.connect_released(move |_, _, x, y| {
        let w = f64::from(pic_for_click.width().max(1));
        let h = f64::from(pic_for_click.height().max(1));
        let vx = (x / w) * 320.0;
        let vy = (y / h) * 520.0;
        if let Some(hit_id) = hit_test_button(svg_for_click.as_deref(), vx, vy) {
            update_svg_picture(&pic_for_click, svg_for_click.as_deref(), Some(&hit_id));
            let desc = btns_for_click
                .iter()
                .find(|b| b.svg_id.as_deref() == Some(hit_id.as_str()))
                .map(|b| format!("{} (CID 0x{:04X})", b.label, b.cid))
                .unwrap_or(hit_id);
            lbl_for_click.set_text(&desc);
        }
    });
    picture.add_controller(click);

    let diagram_box = GtkBox::new(Orientation::Vertical, 8);
    diagram_box.append(&picture);
    diagram_box.append(&selected_label);

    let diagram_row = adw::ActionRow::new();
    diagram_row.set_child(Some(&diagram_box));
    diagram_group.add(&diagram_row);
    page.add(&diagram_group);

    let list_group = adw::PreferencesGroup::new();
    list_group.set_title(tr("Buttons"));
    for b in &buttons_rep.buttons {
        let row = adw::ActionRow::new();
        row.set_title(&b.label);
        row.set_subtitle(&format!("CID 0x{:04X}", b.cid));
        let btn = Button::with_label(tr("Highlight"));
        let pic_ref = picture.clone();
        let svg_ref = svg_name.clone();
        let svg_id_opt = b.svg_id.clone();
        let lbl_ref = selected_label.clone();
        let text = format!("{} (CID 0x{:04X})", b.label, b.cid);
        btn.connect_clicked(move |_| {
            update_svg_picture(&pic_ref, svg_ref.as_deref(), svg_id_opt.as_deref());
            lbl_ref.set_text(&text);
        });
        btn.set_valign(Align::Center);
        row.add_suffix(&btn);
        list_group.add(&row);
    }
    page.add(&list_group);

    page
}

/// Builds the RGB Lighting page (`0x8070`) with an interactive color chooser panel.
#[allow(deprecated)]
pub fn build_lighting_page(snap: &DeviceSnapshot, backend: &BackendHandle) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(tr("Lighting"));
    page.set_icon_name(Some("applications-graphics-symbolic"));

    let Some(led) = &snap.led else {
        return page;
    };

    for zone in &led.zones {
        let group = adw::PreferencesGroup::new();
        group.set_title(&format!("Zone {}: {}", zone.zone_index, zone.location_name));

        let effects = StringList::new(&["off", "static", "breathing", "cycle"]);
        let effect_dropdown = DropDown::new(Some(effects), None::<gtk4::Expression>);
        effect_dropdown.set_selected(1);
        effect_dropdown.set_valign(Align::Center);

        let effect_row = adw::ActionRow::new();
        effect_row.set_title(tr("Effect"));
        effect_row.add_suffix(&effect_dropdown);
        group.add(&effect_row);

        let hex_label = Label::new(Some("#00AAFF"));
        hex_label.add_css_class("monospace");
        hex_label.set_valign(Align::Center);

        let color_row = adw::ActionRow::new();
        color_row.set_title(tr("Color"));
        color_row.add_suffix(&hex_label);
        group.add(&color_row);

        let chooser = ColorChooserWidget::new();
        chooser.set_use_alpha(false);
        chooser.set_show_editor(true);
        chooser.set_rgba(&RGBA::new(0.0, 0.667, 1.0, 1.0));
        chooser.set_margin_top(6);
        chooser.set_margin_bottom(6);

        let hl_clone = hex_label.clone();
        chooser.connect_rgba_notify(move |c| {
            hl_clone.set_text(&rgba_to_hex(&c.rgba()));
        });

        let chooser_row = adw::ActionRow::new();
        chooser_row.set_child(Some(&chooser));
        group.add(&chooser_row);

        let speed_adj = Adjustment::new(2000.0, 500.0, 10000.0, 250.0, 500.0, 0.0);
        let speed_scale = Scale::new(Orientation::Horizontal, Some(&speed_adj));
        speed_scale.set_draw_value(true);
        speed_scale.set_digits(0);
        speed_scale.set_size_request(180, -1);
        let speed_row = adw::ActionRow::new();
        speed_row.set_title(tr("Speed (ms)"));
        speed_row.add_suffix(&speed_scale);
        group.add(&speed_row);

        let bright_adj = Adjustment::new(100.0, 10.0, 100.0, 5.0, 10.0, 0.0);
        let bright_scale = Scale::new(Orientation::Horizontal, Some(&bright_adj));
        bright_scale.set_draw_value(true);
        bright_scale.set_digits(0);
        bright_scale.set_size_request(180, -1);
        let bright_row = adw::ActionRow::new();
        bright_row.set_title(tr("Brightness (%)"));
        bright_row.add_suffix(&bright_scale);
        group.add(&bright_row);

        let apply_btn = Button::with_label(tr("Apply"));
        apply_btn.add_css_class("suggested-action");
        apply_btn.set_valign(Align::Center);

        let key = snap.summary.device_key.clone();
        let zone_idx = zone.zone_index;
        let b = backend.clone();
        let dd = effect_dropdown.clone();
        let ch = chooser.clone();
        let sp = speed_scale.clone();
        let br = bright_scale.clone();
        apply_btn.connect_clicked(move |_| {
            let eff = match dd.selected() {
                0 => "off",
                1 => "static",
                2 => "breathing",
                _ => "cycle",
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let period_ms = sp.value().round().clamp(500.0, 10000.0) as u16;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let brightness_pct = br.value().round().clamp(10.0, 100.0) as u8;
            b.send(UiCommand::SetLedEffect {
                device_key: key.clone(),
                zone_index: zone_idx,
                effect: eff.to_owned(),
                color_hex: rgba_to_hex(&ch.rgba()),
                period_ms,
                brightness_pct,
            });
        });

        let action_row = adw::ActionRow::new();
        action_row.set_title(tr("Apply Effect"));
        action_row.add_suffix(&apply_btn);
        group.add(&action_row);

        page.add(&group);
    }

    page
}

fn rgba_to_hex(c: &RGBA) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let r = (f64::from(c.red()) * 255.0).round().clamp(0.0, 255.0) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let g = (f64::from(c.green()) * 255.0).round().clamp(0.0, 255.0) as u8;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let b = (f64::from(c.blue()) * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// Builds the Onboard Profiles page (`0x8100`) with slot switcher & Backup button.
pub fn build_profiles_page(snap: &DeviceSnapshot, backend: &BackendHandle) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(tr("Profiles"));
    page.set_icon_name(Some("drive-harddisk-symbolic"));

    let Some(profiles) = &snap.profiles else {
        return page;
    };

    let actions_group = adw::PreferencesGroup::new();
    actions_group.set_title("Onboard Memory");
    actions_group.set_description(Some(&format!(
        "{} slots · {} sectors ({} bytes/sector)",
        profiles.profile_count, profiles.sector_count, profiles.sector_size
    )));

    let backup_row = adw::ActionRow::new();
    backup_row.set_title(tr("Backup Now"));
    backup_row.set_subtitle("Save verified CRC-checked snapshot to ~/.local/share/squeak/backups/");
    let backup_btn = Button::with_label(tr("Backup Now"));
    backup_btn.add_css_class("suggested-action");
    backup_btn.set_valign(Align::Center);
    let key = snap.summary.device_key.clone();
    let b = backend.clone();
    backup_btn.connect_clicked(move |_| {
        b.send(UiCommand::BackupProfiles {
            device_key: key.clone(),
        });
    });
    backup_row.add_suffix(&backup_btn);
    actions_group.add(&backup_row);
    page.add(&actions_group);

    let slots_group = adw::PreferencesGroup::new();
    slots_group.set_title(tr("Profiles"));
    for slot in &profiles.profiles {
        let row = adw::ActionRow::new();
        let active_tag = if slot.is_active { " [ACTIVE]" } else { "" };
        row.set_title(&format!("Slot {}: {}{active_tag}", slot.slot, slot.name));

        let stages = slot
            .dpi_stages
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        row.set_subtitle(&format!("{} Hz · DPI [{stages}]", slot.report_rate_hz));

        let activate_btn = Button::with_label(if slot.is_active {
            tr("Active Slot")
        } else {
            tr("Apply")
        });
        activate_btn.set_valign(Align::Center);
        activate_btn.set_sensitive(!slot.is_active);
        if !slot.is_active {
            let key = snap.summary.device_key.clone();
            let b = backend.clone();
            let slot_num = slot.slot;
            activate_btn.connect_clicked(move |_| {
                b.send(UiCommand::SetActiveProfile {
                    device_key: key.clone(),
                    slot: slot_num,
                });
            });
        }
        row.add_suffix(&activate_btn);
        slots_group.add(&row);
    }
    page.add(&slots_group);

    page
}

/// Builds the Scroll Wheel page (`0x2110` SmartShift / `0x2121` HiRes Wheel).
pub fn build_scroll_page(snap: &DeviceSnapshot) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::new();
    page.set_title(tr("Scroll Wheel"));
    page.set_icon_name(Some("input-mouse-symbolic"));

    let Some(scroll) = &snap.scroll else {
        return page;
    };

    if scroll.smart_shift_supported {
        let group = adw::PreferencesGroup::new();
        group.set_title("SmartShift (0x2110)");
        if let Some(mode) = &scroll.smart_shift_mode {
            add_kv_row(&group, "Mode", mode);
        }
        if let Some(thresh) = scroll.auto_disengage {
            add_kv_row(&group, "Auto-disengage threshold", &thresh.to_string());
        }
        page.add(&group);
    }

    if scroll.hires_wheel_supported {
        let group = adw::PreferencesGroup::new();
        group.set_title("Hi-Res Wheel (0x2121)");
        if let Some(mult) = scroll.hires_multiplier {
            add_kv_row(&group, "Multiplier", &format!("{mult}x"));
        }
        if let Some(mode) = &scroll.wheel_resolution_mode {
            add_kv_row(&group, "Resolution mode", mode);
        }
        if let Some(inv) = scroll.invert {
            add_kv_row(&group, "Inverted", if inv { "Yes" } else { "No" });
        }
        page.add(&group);
    }

    page
}

/// Builds an actionable `AdwStatusPage` for permission, disconnect, or daemon errors.
pub fn build_status_error_page(err: &ErrorViewState, backend: &BackendHandle) -> adw::StatusPage {
    let status = adw::StatusPage::new();
    match err {
        ErrorViewState::NoHidrawPermission { device_name } => {
            status.set_icon_name(Some("dialog-password-symbolic"));
            status.set_title(tr("No permission to access HID++ device"));
            status.set_description(Some(&format!(
                "{device_name} requires a udev uaccess rule to allow user-space access."
            )));
            let btn = Button::with_label(tr("Install udev rule"));
            btn.add_css_class("suggested-action");
            btn.add_css_class("pill");
            btn.set_halign(Align::Center);
            let b = backend.clone();
            btn.connect_clicked(move |_| {
                b.send(UiCommand::InstallUdevRule);
            });
            status.set_child(Some(&btn));
        }
        ErrorViewState::DaemonNotRunning { detail } => {
            status.set_icon_name(Some("network-offline-symbolic"));
            status.set_title(tr("Daemon not running"));
            status.set_description(Some(&format!(
                "Could not connect to dev.squeak.Daemon ({detail}). Start `squeakd` and retry."
            )));
            let btn = Button::with_label(tr("Retry"));
            btn.add_css_class("suggested-action");
            btn.add_css_class("pill");
            btn.set_halign(Align::Center);
            let b = backend.clone();
            btn.connect_clicked(move |_| {
                b.send(UiCommand::RefreshAll { selected_key: None });
            });
            status.set_child(Some(&btn));
        }
        ErrorViewState::DeviceAsleepOrDisconnected { device_name } => {
            status.set_icon_name(Some("input-mouse-symbolic"));
            status.set_title(tr("Device disconnected or asleep"));
            status.set_description(Some(&format!(
                "{device_name} is currently unreachable or asleep. Wake or reconnect the device."
            )));
            let btn = Button::with_label(tr("Retry"));
            btn.add_css_class("suggested-action");
            btn.add_css_class("pill");
            btn.set_halign(Align::Center);
            let b = backend.clone();
            btn.connect_clicked(move |_| {
                b.send(UiCommand::RefreshAll { selected_key: None });
            });
            status.set_child(Some(&btn));
        }
        ErrorViewState::UnsupportedFeature { feature_name } => {
            status.set_icon_name(Some("dialog-information-symbolic"));
            status.set_title(tr("Unsupported feature"));
            status.set_description(Some(feature_name));
        }
        ErrorViewState::Other { detail } => {
            status.set_icon_name(Some("dialog-warning-symbolic"));
            status.set_title(tr("Squeak"));
            status.set_description(Some(detail));
            let btn = Button::with_label(tr("Retry"));
            btn.add_css_class("pill");
            btn.set_halign(Align::Center);
            let b = backend.clone();
            btn.connect_clicked(move |_| {
                b.send(UiCommand::RefreshAll { selected_key: None });
            });
            status.set_child(Some(&btn));
        }
    }
    status
}

fn add_kv_row(group: &adw::PreferencesGroup, title: &str, value: &str) {
    let row = adw::ActionRow::new();
    row.set_title(title);
    let lbl = Label::new(Some(value));
    lbl.set_selectable(true);
    lbl.add_css_class("dim-label");
    row.add_suffix(&lbl);
    group.add(&row);
}

fn update_svg_picture(picture: &Picture, svg_name: Option<&str>, selected_svg_id: Option<&str>) {
    let svg_xml = render_svg_with_highlight(svg_name, selected_svg_id);
    let loader = PixbufLoader::new();
    if loader.write(svg_xml.as_bytes()).is_ok()
        && loader.close().is_ok()
        && let Some(pixbuf) = loader.pixbuf()
    {
        let texture = Texture::for_pixbuf(&pixbuf);
        picture.set_paintable(Some(&texture));
    }
}

#[derive(Clone)]
struct PresetRebuildCtx {
    flow: FlowBox,
    presets: Rc<RefCell<Vec<u16>>>,
    device_key: String,
    sensor_index: u8,
    backend: BackendHandle,
    scale: Scale,
    value_label: Label,
    suppress_slider: Rc<RefCell<bool>>,
    last_sent_dpi: Rc<RefCell<u16>>,
}

fn rebuild_preset_buttons(ctx: &PresetRebuildCtx) {
    while let Some(child) = ctx.flow.first_child() {
        ctx.flow.remove(&child);
    }
    let list = ctx.presets.borrow().clone();
    let active_dpi = *ctx.last_sent_dpi.borrow();
    let can_remove = list.len() > 1;

    for dpi_val in list {
        let pill = GtkBox::new(Orientation::Horizontal, 0);
        pill.add_css_class("linked");

        let apply_btn = Button::with_label(&format!("{dpi_val} DPI"));
        if dpi_val == active_dpi {
            apply_btn.add_css_class("suggested-action");
        }
        {
            let c = ctx.clone();
            apply_btn.connect_clicked(move |_| {
                *c.suppress_slider.borrow_mut() = true;
                c.scale.set_value(f64::from(dpi_val));
                c.value_label.set_text(&format!("{dpi_val} DPI"));
                *c.suppress_slider.borrow_mut() = false;
                *c.last_sent_dpi.borrow_mut() = dpi_val;
                rebuild_preset_buttons(&c);
                c.backend.send(UiCommand::SetDpi {
                    device_key: c.device_key.clone(),
                    sensor_index: c.sensor_index,
                    dpi: dpi_val,
                });
            });
        }
        pill.append(&apply_btn);

        if can_remove {
            let rm_btn = Button::with_label("×");
            rm_btn.set_tooltip_text(Some("Remove preset"));
            let c = ctx.clone();
            rm_btn.connect_clicked(move |_| {
                c.presets.borrow_mut().retain(|&v| v != dpi_val);
                rebuild_preset_buttons(&c);
            });
            pill.append(&rm_btn);
        }

        ctx.flow.insert(&pill, -1);
    }
}
