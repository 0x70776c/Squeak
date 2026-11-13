//! Main GTK4 / libadwaita application window, navigation shell, and async event loop.

pub mod pages;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DropDown, Label, ListBox, ListBoxRow, Orientation, SelectionMode,
    Spinner, Stack, StackTransitionType, StringList,
};
use libadwaita as adw;
use squeak_core::DaemonDeviceSummary;

use crate::backend::{BackendHandle, DeviceSnapshot, UiCommand, UiEvent};
use crate::i18n::tr;
use crate::state::{
    DeviceCapabilities, ErrorViewState, UndoAction, UndoStack, classify_device_probe,
};

/// Application ID matching Desktop & Flatpak metadata.
pub const APP_ID: &str = "dev.squeak.Squeak";

#[derive(Default)]
struct UiState {
    devices: Vec<DaemonDeviceSummary>,
    current_key: Option<String>,
    current_snapshot: Option<DeviceSnapshot>,
    undo: UndoStack,
}

/// Launches the `libadwaita` GUI application.
pub fn launch_gui() -> gtk4::glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let (backend, evt_rx) = BackendHandle::spawn();
    let state = Rc::new(RefCell::new(UiState::default()));

    let toast_overlay = adw::ToastOverlay::new();
    let content_stack = Stack::new();
    content_stack.set_transition_type(StackTransitionType::Crossfade);
    content_stack.set_hexpand(true);
    content_stack.set_vexpand(true);

    let nav_list = ListBox::new();
    nav_list.set_selection_mode(SelectionMode::Single);
    nav_list.add_css_class("navigation-sidebar");

    let device_dropdown = DropDown::new(Some(StringList::new(&[])), None::<gtk4::Expression>);
    device_dropdown.set_hexpand(true);

    let spinner = Spinner::new();
    spinner.set_spinning(true);

    let undo_btn = Button::from_icon_name("edit-undo-symbolic");
    undo_btn.set_tooltip_text(Some(tr("Undo")));
    undo_btn.set_sensitive(false);

    let refresh_btn = Button::from_icon_name("view-refresh-symbolic");
    refresh_btn.set_tooltip_text(Some(tr("Refresh")));

    // Sidebar layout
    let sidebar_box = GtkBox::new(Orientation::Vertical, 0);
    let sidebar_header = adw::HeaderBar::new();
    sidebar_header.set_title_widget(Some(&adw::WindowTitle::new(tr("Squeak"), "")));
    sidebar_header.pack_end(&refresh_btn);
    sidebar_box.append(&sidebar_header);

    let dev_picker_box = GtkBox::new(Orientation::Horizontal, 6);
    dev_picker_box.set_margin_start(12);
    dev_picker_box.set_margin_end(12);
    dev_picker_box.set_margin_top(8);
    dev_picker_box.set_margin_bottom(8);
    dev_picker_box.append(&device_dropdown);
    sidebar_box.append(&dev_picker_box);
    sidebar_box.append(&nav_list);

    // Content pane layout
    let content_box = GtkBox::new(Orientation::Vertical, 0);
    let content_header = adw::HeaderBar::new();
    let content_title = adw::WindowTitle::new(tr("Overview"), "");
    content_header.set_title_widget(Some(&content_title));
    content_header.pack_start(&spinner);
    content_header.pack_end(&undo_btn);
    content_box.append(&content_header);
    content_box.append(&content_stack);

    let root_widget = build_split_container(&sidebar_box, &content_box);
    toast_overlay.set_child(Some(&root_widget));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(tr("Squeak"))
        .default_width(920)
        .default_height(640)
        .content(&toast_overlay)
        .build();

    // Wire Refresh button
    {
        let b = backend.clone();
        let st = Rc::clone(&state);
        let sp = spinner.clone();
        refresh_btn.connect_clicked(move |_| {
            sp.set_spinning(true);
            let sel = st.borrow().current_key.clone();
            b.send(UiCommand::RefreshAll { selected_key: sel });
        });
    }

    // Wire Undo button
    {
        let b = backend.clone();
        let st = Rc::clone(&state);
        let ub = undo_btn.clone();
        let to = toast_overlay.clone();
        undo_btn.connect_clicked(move |_| {
            let action = st.borrow_mut().undo.pop();
            ub.set_sensitive(st.borrow().undo.can_undo());
            if let Some(act) = action {
                match act {
                    UndoAction::Dpi {
                        device_key,
                        sensor_index,
                        previous_dpi,
                    } => b.send(UiCommand::SetDpi {
                        device_key,
                        sensor_index,
                        dpi: previous_dpi,
                    }),
                    UndoAction::ReportRate {
                        device_key,
                        previous_hz,
                    } => b.send(UiCommand::SetReportRate {
                        device_key,
                        hz: previous_hz,
                    }),
                    UndoAction::OnboardMode {
                        device_key,
                        previous_mode,
                    } => b.send(UiCommand::SetOnboardMode {
                        device_key,
                        mode: previous_mode,
                    }),
                    UndoAction::ActiveProfile {
                        device_key,
                        previous_slot,
                    } => b.send(UiCommand::SetActiveProfile {
                        device_key,
                        slot: previous_slot,
                    }),
                }
                to.add_toast(adw::Toast::new(tr("Change undone")));
            }
        });
    }

    // Wire Device dropdown selection
    {
        let b = backend.clone();
        let st = Rc::clone(&state);
        let sp = spinner.clone();
        device_dropdown.connect_selected_notify(move |dd| {
            let idx = dd.selected() as usize;
            let key_opt = st.borrow().devices.get(idx).map(|d| d.device_key.clone());
            if let Some(key) = key_opt
                && st.borrow().current_key.as_deref() != Some(key.as_str())
            {
                st.borrow_mut().current_key = Some(key.clone());
                sp.set_spinning(true);
                b.send(UiCommand::LoadDevice { device_key: key });
            }
        });
    }

    // Wire Sidebar navigation selection
    {
        let cs = content_stack.clone();
        let ct = content_title.clone();
        nav_list.connect_row_selected(move |_, row_opt| {
            if let Some(row) = row_opt {
                let name = row.widget_name();
                cs.set_visible_child_name(name.as_str());
                ct.set_title(&page_title_for_name(name.as_str()));
            }
        });
    }

    // Initial load
    backend.send(UiCommand::RefreshAll { selected_key: None });

    // Non-blocking event pump (polls std::sync::mpsc::Receiver every 25 ms)
    let b_pump = backend.clone();
    let st_pump = Rc::clone(&state);
    let to_pump = toast_overlay.clone();
    let cs_pump = content_stack.clone();
    let nl_pump = nav_list.clone();
    let dd_pump = device_dropdown.clone();
    let sp_pump = spinner.clone();
    let ub_pump = undo_btn.clone();

    glib::timeout_add_local(Duration::from_millis(25), move || {
        while let Ok(evt) = evt_rx.try_recv() {
            handle_ui_event(
                evt, &b_pump, &st_pump, &to_pump, &cs_pump, &nl_pump, &dd_pump, &sp_pump, &ub_pump,
            );
        }
        glib::ControlFlow::Continue
    });

    window.present();
}

#[cfg(feature = "adw_v1_4")]
fn build_split_container(sidebar: &GtkBox, content: &GtkBox) -> gtk4::Widget {
    let split = adw::NavigationSplitView::new();
    let sidebar_page = adw::NavigationPage::new(sidebar, tr("Devices"));
    let content_page = adw::NavigationPage::new(content, tr("Squeak"));
    split.set_sidebar(Some(&sidebar_page));
    split.set_content(Some(&content_page));
    split.upcast()
}

#[cfg(not(feature = "adw_v1_4"))]
fn build_split_container(sidebar: &GtkBox, content: &GtkBox) -> gtk4::Widget {
    let leaflet = adw::Leaflet::new();
    sidebar.set_size_request(250, -1);
    leaflet.append(sidebar);
    leaflet.append(content);
    leaflet.upcast()
}

#[allow(clippy::too_many_arguments)]
fn handle_ui_event(
    evt: UiEvent,
    backend: &BackendHandle,
    state: &Rc<RefCell<UiState>>,
    toast_overlay: &adw::ToastOverlay,
    content_stack: &Stack,
    nav_list: &ListBox,
    device_dropdown: &DropDown,
    spinner: &Spinner,
    undo_btn: &Button,
) {
    match evt {
        UiEvent::DevicesLoaded { devices, snapshot } => {
            spinner.set_spinning(false);
            let names: Vec<&str> = devices.iter().map(|d| d.matched_name.as_str()).collect();
            let model = StringList::new(&names);
            device_dropdown.set_model(Some(&model));

            {
                let mut st = state.borrow_mut();
                st.devices = devices;
                st.current_key = snapshot.as_ref().map(|s| s.summary.device_key.clone());
                st.current_snapshot = snapshot.as_deref().cloned();
            }

            if let Some(snap) = snapshot {
                render_snapshot(&snap, backend, content_stack, nav_list);
            } else {
                let empty = ErrorViewState::DeviceAsleepOrDisconnected {
                    device_name: "No HID++ devices".to_owned(),
                };
                show_error_page(&empty, backend, content_stack, nav_list);
            }
        }
        UiEvent::DeviceSnapshotLoaded(snap) => {
            spinner.set_spinning(false);
            state.borrow_mut().current_snapshot = Some((*snap).clone());
            render_snapshot(&snap, backend, content_stack, nav_list);
        }
        UiEvent::Error(err) => {
            spinner.set_spinning(false);
            show_error_page(&err, backend, content_stack, nav_list);
        }
        UiEvent::DpiApplied(rep) => {
            if rep.applied && rep.verified {
                if let Some(key) = state.borrow().current_key.clone()
                    && rep.previous_dpi != rep.requested_dpi
                {
                    state.borrow_mut().undo.push(UndoAction::Dpi {
                        device_key: key,
                        sensor_index: rep.sensor_index,
                        previous_dpi: rep.previous_dpi,
                    });
                    undo_btn.set_sensitive(true);
                }
                toast_overlay.add_toast(adw::Toast::new(&format!(
                    "{}: {} DPI",
                    tr("Settings applied"),
                    rep.requested_dpi
                )));
            }
        }
        UiEvent::ReportRateApplied(rep) => {
            if rep.applied && rep.verified {
                if let Some(key) = state.borrow().current_key.clone()
                    && rep.previous_hz != rep.requested_hz
                {
                    state.borrow_mut().undo.push(UndoAction::ReportRate {
                        device_key: key,
                        previous_hz: rep.previous_hz,
                    });
                    undo_btn.set_sensitive(true);
                }
                toast_overlay.add_toast(adw::Toast::new(&format!(
                    "{}: {} Hz",
                    tr("Settings applied"),
                    rep.requested_hz
                )));
                if let Some(key) = state.borrow().current_key.clone() {
                    backend.send(UiCommand::LoadDevice { device_key: key });
                }
            }
        }
        UiEvent::LedApplied(rep) => {
            if rep.applied {
                toast_overlay.add_toast(adw::Toast::new(&format!(
                    "{}: {} ({})",
                    tr("Settings applied"),
                    rep.effect,
                    rep.zone_index
                )));
            }
        }
        UiEvent::OnboardModeApplied(rep) => {
            if rep.applied && rep.verified {
                if let Some(key) = state.borrow().current_key.clone()
                    && rep.previous_mode != rep.requested_mode
                {
                    state.borrow_mut().undo.push(UndoAction::OnboardMode {
                        device_key: key.clone(),
                        previous_mode: rep.previous_mode,
                    });
                    undo_btn.set_sensitive(true);
                }
                toast_overlay.add_toast(adw::Toast::new(&format!(
                    "{}: {}",
                    tr("Operating Mode"),
                    rep.requested_mode
                )));
                if let Some(key) = state.borrow().current_key.clone() {
                    backend.send(UiCommand::LoadDevice { device_key: key });
                }
            }
        }
        UiEvent::ActiveProfileApplied(rep) => {
            if rep.applied && rep.verified {
                if let Some(key) = state.borrow().current_key.clone()
                    && rep.previous_slot != rep.requested_slot
                {
                    state.borrow_mut().undo.push(UndoAction::ActiveProfile {
                        device_key: key,
                        previous_slot: rep.previous_slot,
                    });
                    undo_btn.set_sensitive(true);
                }
                toast_overlay.add_toast(adw::Toast::new(&format!(
                    "{}: Slot {}",
                    tr("Active Slot"),
                    rep.requested_slot
                )));
                if let Some(key) = state.borrow().current_key.clone() {
                    backend.send(UiCommand::LoadDevice { device_key: key });
                }
            }
        }
        UiEvent::BackupCreated(rep) => {
            toast_overlay.add_toast(adw::Toast::new(&format!(
                "{}: {}",
                tr("Backup created"),
                rep.backup_path
            )));
        }
        UiEvent::UdevInstallFinished(res) => match res {
            Ok(()) => {
                toast_overlay.add_toast(adw::Toast::new("udev rule installed"));
                backend.send(UiCommand::RefreshAll { selected_key: None });
            }
            Err(msg) => {
                toast_overlay.add_toast(adw::Toast::new(&msg));
            }
        },
        UiEvent::ToastMessage(msg) => {
            toast_overlay.add_toast(adw::Toast::new(&msg));
        }
    }
}

fn render_snapshot(
    snap: &DeviceSnapshot,
    backend: &BackendHandle,
    content_stack: &Stack,
    nav_list: &ListBox,
) {
    if let Some(err) = classify_device_probe(&snap.summary) {
        show_error_page(&err, backend, content_stack, nav_list);
        return;
    }

    let previous_page = content_stack
        .visible_child_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "overview".to_owned());

    clear_stack_and_nav(content_stack, nav_list);

    let caps = DeviceCapabilities::from_reports(
        snap.status.as_ref(),
        snap.buttons.as_ref(),
        snap.led.as_ref(),
        snap.profiles.as_ref(),
        snap.scroll.as_ref(),
    );

    add_page(
        content_stack,
        nav_list,
        "overview",
        tr("Overview"),
        &pages::build_overview_page(snap, backend),
    );
    if caps.has_dpi_or_rate {
        add_page(
            content_stack,
            nav_list,
            "dpi",
            tr("DPI & Sensor"),
            &pages::build_dpi_page(snap, backend),
        );
    }
    if caps.has_buttons {
        add_page(
            content_stack,
            nav_list,
            "buttons",
            tr("Buttons"),
            &pages::build_buttons_page(snap),
        );
    }
    if caps.has_lighting {
        add_page(
            content_stack,
            nav_list,
            "lighting",
            tr("Lighting"),
            &pages::build_lighting_page(snap, backend),
        );
    }
    if caps.has_profiles {
        add_page(
            content_stack,
            nav_list,
            "profiles",
            tr("Profiles"),
            &pages::build_profiles_page(snap, backend),
        );
    }
    if caps.has_scroll {
        add_page(
            content_stack,
            nav_list,
            "scroll",
            tr("Scroll Wheel"),
            &pages::build_scroll_page(snap),
        );
    }

    if content_stack.child_by_name(&previous_page).is_some() {
        content_stack.set_visible_child_name(&previous_page);
    } else {
        content_stack.set_visible_child_name("overview");
    }
    if let Some(first_row) = nav_list.row_at_index(0) {
        nav_list.select_row(Some(&first_row));
    }
}

fn show_error_page(
    err: &ErrorViewState,
    backend: &BackendHandle,
    content_stack: &Stack,
    nav_list: &ListBox,
) {
    clear_stack_and_nav(content_stack, nav_list);
    let page = pages::build_status_error_page(err, backend);
    content_stack.add_named(&page, Some("error"));
    content_stack.set_visible_child_name("error");
}

fn clear_stack_and_nav(stack: &Stack, nav: &ListBox) {
    while let Some(child) = stack.first_child() {
        stack.remove(&child);
    }
    while let Some(row) = nav.first_child() {
        nav.remove(&row);
    }
}

fn add_page(stack: &Stack, nav: &ListBox, name: &str, title: &str, page: &impl IsA<gtk4::Widget>) {
    stack.add_named(page, Some(name));
    let row = ListBoxRow::new();
    row.set_widget_name(name);
    let lbl = Label::new(Some(title));
    lbl.set_halign(Align::Start);
    lbl.set_margin_start(12);
    lbl.set_margin_end(12);
    lbl.set_margin_top(10);
    lbl.set_margin_bottom(10);
    row.set_child(Some(&lbl));
    nav.append(&row);
}

fn page_title_for_name(name: &str) -> String {
    match name {
        "overview" => tr("Overview").to_owned(),
        "dpi" => tr("DPI & Sensor").to_owned(),
        "buttons" => tr("Buttons").to_owned(),
        "lighting" => tr("Lighting").to_owned(),
        "profiles" => tr("Profiles").to_owned(),
        "scroll" => tr("Scroll Wheel").to_owned(),
        _ => tr("Squeak").to_owned(),
    }
}
