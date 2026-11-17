use crate::{
    audio::Audio,
    camera::{self, Worker},
    focus::{self, AlertGate, Attention, Features, NAMES, Zone},
    settings::{self, Settings, SoundSite},
    tray::{Command, TrayIcon},
};
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

struct Calibration {
    zone: usize,
    started: Instant,
    samples: Vec<Features>,
}
struct State {
    settings: Settings,
    path: PathBuf,
    worker: Option<Worker>,
    audio: Audio,
    gate: AlertGate,
    calibration: Option<Calibration>,
    clock: Instant,
    last_result: Instant,
    blocked: bool,
}
impl State {
    fn save(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.blocked,
            "Saved settings could not be loaded; original file has been preserved."
        );
        self.settings.save(&self.path)
    }
    fn pause(&mut self) {
        self.worker.take();
        self.calibration = None;
        self.reset_alert();
    }
    fn reset_alert(&mut self) {
        self.gate.reset();
        self.audio.stop();
    }
    fn sound(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.settings.sound {
            return Ok(path.clone());
        }
        let path = settings::data_directory().join("nudge.wav");
        if !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, include_bytes!("../assets/nudge.wav"))?;
        }
        Ok(path)
    }
}
fn rebuild_sound_sites(list: &gtk::Box, state: &Rc<RefCell<State>>, notice: &gtk::Label) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
    for (index, site) in state
        .borrow()
        .settings
        .sound_sites
        .clone()
        .into_iter()
        .enumerate()
    {
        let website = row(8);
        let link = gtk::LinkButton::with_label(&site.url, &site.label);
        link.set_tooltip_text(Some(&site.url));
        link.set_hexpand(true);
        link.set_halign(gtk::Align::Start);
        website.append(&link);
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.set_tooltip_text(Some(&format!("Remove {}", site.label)));
        website.append(&remove);
        remove.connect_clicked({
            let (state, notice, list) = (state.clone(), notice.clone(), list.downgrade());
            move |_| {
                let mut state_ref = state.borrow_mut();
                if index >= state_ref.settings.sound_sites.len() {
                    return;
                }
                state_ref.settings.sound_sites.remove(index);
                let result = state_ref.save();
                drop(state_ref);
                feedback(&notice, result, "Website link removed.");
                if let Some(list) = list.upgrade() {
                    rebuild_sound_sites(&list, &state, &notice);
                }
            }
        });
        list.append(&website);
    }
}
fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.);
    label.set_wrap(true);
    label.add_css_class(class);
    label
}
fn column(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, spacing)
}
fn row(spacing: i32) -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, spacing)
}
fn card(title: &str) -> gtk::Box {
    let card = column(12);
    card.add_css_class("card");
    card.append(&label(title, "section-title"));
    card
}
fn feedback(notice: &gtk::Label, result: anyhow::Result<()>, success: &str) {
    notice.set_text(&match result {
        Ok(()) => success.into(),
        Err(error) => format!("{error:#}"),
    });
}
fn accent_foreground(accent: &str) -> &'static str {
    let rgb = u32::from_str_radix(accent.trim_start_matches('#'), 16).unwrap_or(0xa8d5ba);
    let linear = |value: u32| {
        let v = value as f64 / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear((rgb >> 16) & 255)
        + 0.7152 * linear((rgb >> 8) & 255)
        + 0.0722 * linear(rgb & 255);
    if luminance > 0.179 { "black" } else { "white" }
}
fn theme_css() -> String {
    let colors = std::fs::read_to_string(
        dirs::home_dir()
            .unwrap_or_default()
            .join(".local/state/omarchy/current/theme/colors.toml"),
    )
    .unwrap_or_default();
    let accent = colors
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.trim() != "accent" {
                return None;
            }
            let value = value.trim().trim_matches(['\'', '"']);
            (value.len() == 7
                && value.starts_with('#')
                && value[1..].chars().all(|c| c.is_ascii_hexdigit()))
            .then_some(value)
        })
        .unwrap_or("#a8d5ba");
    let foreground = accent_foreground(accent);
    format!(
        r#"
window {{ background: #10161b; color: #edf1ef; }}
.root {{ padding: 20px; }} .title {{ font-size: 30px; font-weight: 800; letter-spacing: -0.5px; }}
.eyebrow {{ color: {accent}; font-size: 10px; font-weight: 700; letter-spacing: 2px; }}
.muted {{ color: #a8b7bf; font-size: 12px; }} .section-title {{ font-size: 15px; font-weight: 700; }}
.status {{ font-size: 22px; font-weight: 700; }} .status.alerting {{ color: #f0c88c; }}
.card {{ background: #182127; padding: 16px; border: 1px solid #2c393f; border-radius: 8px; }}
button {{ background: #26343d; color: #edf1ef; padding: 9px 13px; border: 1px solid #3c4b55; box-shadow: none; text-shadow: none; }}
button:hover {{ background: #30434e; }} button:disabled {{ opacity: 0.55; }}
button.primary {{ background: {accent}; color: {foreground}; border-color: {accent}; font-weight: 700; }}
button.primary:hover {{ opacity: 0.9; }} button.link {{ background: transparent; border: none; padding: 5px 0; }} button.link label {{ color: {accent}; }}
entry, spinbutton {{ background: #10191f; color: #edf1ef; border: 1px solid #3c4b55; }}
spinbutton button {{ padding: 6px 10px; }}
button:focus-visible, entry:focus {{ outline: 2px solid {accent}; outline-offset: 2px; }}
switch {{ background: #34444e; }} switch:checked {{ background: {accent}; }} switch slider {{ background: #edf1ef; }}
.notice {{ color: #e3c698; font-size: 12px; }} progressbar trough {{ background: #34444e; border: none; min-height: 4px; }} progressbar progress {{ background: {accent}; }}
.preview {{ background: #0c1115; border: 1px solid #2c393f; }}
"#
    )
}

pub fn launch(smoke: bool) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("dev.focusbuddy.FocusBuddy")
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.windows().first() {
            window.present();
            return;
        }
        build(app, smoke);
    });
    app.run_with_args::<&str>(&[])
}
fn build(app: &gtk::Application, smoke: bool) {
    let path = settings::data_directory().join("state.json");
    let loaded = Settings::load(&path);
    let warning = loaded
        .as_ref()
        .err()
        .map(|error| format!("{error:#} File: {}", path.display()));
    let state = Rc::new(RefCell::new(State {
        blocked: loaded.is_err(),
        settings: loaded.unwrap_or_default(),
        path,
        worker: None,
        audio: Audio::default(),
        gate: AlertGate::default(),
        calibration: None,
        clock: Instant::now(),
        last_result: Instant::now(),
    }));
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Focus Buddy")
        .default_width(1000)
        .default_height(780)
        .build();
    let provider = gtk::CssProvider::new();
    let mut current_css = theme_css();
    provider.load_from_data(&current_css);
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let theme_timer = glib::timeout_add_local(Duration::from_secs(5), move || {
        let css = theme_css();
        if css != current_css {
            provider.load_from_data(&css);
            current_css = css;
        }
        glib::ControlFlow::Continue
    });
    let root = column(20);
    root.add_css_class("root");
    root.append(&label("A LITTLE NUDGE, BACK TO WORK", "eyebrow"));
    let heading = row(16);
    let titles = column(5);
    titles.set_hexpand(true);
    titles.append(&label("Focus Buddy", "title"));
    titles.append(&label(
        "A small reminder to come back to what matters.",
        "muted",
    ));
    heading.append(&titles);
    let start = gtk::Button::with_label("Start session");
    start.add_css_class("primary");
    heading.append(&start);
    let background = gtk::Button::with_label("Hide to tray");
    background.set_tooltip_text(Some(
        "Keep the session running; reopen from the tray or app launcher",
    ));
    heading.append(&background);
    let quit = gtk::Button::with_label("Quit");
    quit.set_tooltip_text(Some("Stop monitoring and exit Focus Buddy"));
    heading.append(&quit);
    quit.connect_clicked({
        let app = app.clone();
        move |_| app.quit()
    });
    root.append(&heading);
    let body = row(20);
    body.set_homogeneous(true);
    let left = column(16);
    left.set_hexpand(true);
    let right = column(16);
    right.set_width_request(400);
    let live = card("Your session");
    let status = label("Paused · camera is off", "status");
    live.append(&status);
    let progress = gtk::ProgressBar::new();
    live.append(&progress);
    let metrics = label(
        "10 FPS analysis cap · one inference thread per model",
        "muted",
    );
    let setup_hint = label(
        "First time? Start setup, then set the positions of both screens.",
        "muted",
    );
    live.append(&setup_hint);
    let technical = gtk::Expander::new(Some("Technical details"));
    technical.set_child(Some(&metrics));
    live.append(&technical);
    left.append(&live);
    let preview_card = card("Camera");
    let available = Rc::new(RefCell::new(camera::cameras()));
    let names: Vec<String> = available
        .borrow()
        .iter()
        .enumerate()
        .map(|(index, (_, name))| {
            format!(
                "{} · Camera {}",
                name.split(" · /dev/").next().unwrap_or(name),
                index + 1
            )
        })
        .collect();
    let names_ref: Vec<&str> = names.iter().map(String::as_str).collect();
    let cameras = gtk::DropDown::from_strings(&names_ref);
    cameras.update_property(&[gtk::accessible::Property::Label("Camera")]);
    let selected = available
        .borrow()
        .iter()
        .position(|(path, _)| *path == state.borrow().settings.camera);
    if let Some(index) = selected {
        cameras.set_selected(index as u32);
    } else if let Some((path, _)) = available.borrow().first() {
        state.borrow_mut().settings.camera = path.clone();
    }
    cameras.set_hexpand(true);
    let refresh = gtk::Button::with_label("Refresh");
    let camera_row = row(8);
    camera_row.append(&cameras);
    camera_row.append(&refresh);
    preview_card.append(&camera_row);
    let picture = gtk::Picture::new();
    picture.set_size_request(360, 240);
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.add_css_class("preview");
    picture.set_visible(false);
    preview_card.append(&picture);
    let preview_toggle = gtk::CheckButton::with_label("Show camera preview");
    preview_card.append(&preview_toggle);
    preview_card.append(&label(
        "Preview is off by default. Pausing releases the camera.",
        "muted",
    ));
    left.append(&preview_card);
    let calibration = card("Set up your desk");
    calibration.append(&label("Start setup, look at Screen 1, then choose Set position. Repeat for Screen 2. Each step takes about 6 seconds.","muted"));
    let mut zone_buttons = Vec::new();
    let mut notebook_row = None;
    for (i, name) in NAMES.iter().enumerate() {
        let zone_row = row(10);
        let text = label(name, "section-title");
        text.set_hexpand(true);
        zone_row.append(&text);
        let button = gtk::Button::with_label(if state.borrow().settings.zones[i].is_some() {
            "Update position"
        } else {
            "Set position"
        });
        zone_row.append(&button);
        if i == 2 {
            zone_row.set_visible(state.borrow().settings.notes);
            notebook_row = Some(zone_row.clone());
        }
        calibration.append(&zone_row);
        zone_buttons.push(button);
    }
    calibration.append(&label("Set both screens before reminders begin. Add your notebook or phone position if you use them. Keep your phone away from your notebook.","muted"));
    let notebook_row = notebook_row.unwrap();
    let clear = gtk::Button::with_label("Reset saved positions…");
    calibration.append(&clear);
    right.append(&calibration);
    let sound_card = card("Reminder sound");
    let sound_name = label(
        &state
            .borrow()
            .settings
            .sound
            .as_ref()
            .map_or("Default chime".into(), |p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }),
        "muted",
    );
    sound_card.append(&sound_name);
    let sound_row = row(8);
    let choose = gtk::Button::with_label("Choose sound…");
    let test = gtk::Button::with_label("Preview sound");
    let default_sound = gtk::Button::with_label("Use chime");
    sound_row.append(&choose);
    sound_row.append(&test);
    sound_row.append(&default_sound);
    sound_card.append(&sound_row);
    sound_card.append(&label("Find a sound you like", "section-title"));
    let sound_sites = column(4);
    sound_card.append(&sound_sites);
    sound_card.append(&label(
        "Open a website, download an audio file, then choose it above.",
        "muted",
    ));
    let site_editor = gtk::Expander::new(Some("Add a website"));
    let editor = column(8);
    let site_name = gtk::Entry::new();
    site_name.set_placeholder_text(Some("Website name"));
    site_name.update_property(&[gtk::accessible::Property::Label("Website name")]);
    let site_url = gtk::Entry::new();
    site_url.set_placeholder_text(Some("https://example.com/sounds"));
    site_url.update_property(&[gtk::accessible::Property::Label("Website URL")]);
    let add_site = gtk::Button::with_label("Save website link");
    editor.append(&site_name);
    editor.append(&site_url);
    editor.append(&add_site);
    site_editor.set_child(Some(&editor));
    sound_card.append(&site_editor);
    let settings_card = card("Settings");
    let notes_row = row(16);
    let mode_title = column(4);
    mode_title.set_hexpand(true);
    mode_title.append(&label("Notebook mode", "section-title"));
    mode_title.append(&label(
        "Allow handwritten notes. Turning this off keeps your saved position.",
        "muted",
    ));
    notes_row.append(&mode_title);
    let notes = gtk::Switch::new();
    notes.set_valign(gtk::Align::Center);
    notes.update_property(&[gtk::accessible::Property::Label("Notebook mode")]);
    notes.set_active(state.borrow().settings.notes);
    notes.set_tooltip_text(Some(
        "Allow a calibrated notebook position during this session",
    ));
    notes_row.append(&notes);
    settings_card.append(&notes_row);
    let timing = row(8);
    let timing_label = label("Remind me after (seconds)", "muted");
    timing_label.set_hexpand(true);
    timing.append(&timing_label);
    let delay = gtk::SpinButton::with_range(2., 30., 1.);
    delay.update_property(&[gtk::accessible::Property::Label(
        "Reminder delay in seconds",
    )]);
    delay.set_value(state.borrow().settings.delay as f64);
    timing.append(&delay);
    settings_card.append(&timing);
    let tolerance_row = row(8);
    let tolerance_label = label("Movement allowance", "muted");
    tolerance_label.set_hexpand(true);
    tolerance_row.append(&tolerance_label);
    let tolerance = gtk::SpinButton::with_range(1.5, 4., 0.1);
    tolerance.update_property(&[gtk::accessible::Property::Label("Movement allowance")]);
    tolerance.set_digits(1);
    tolerance.set_value(state.borrow().settings.tolerance);
    tolerance_row.append(&tolerance);
    let fine_tune = gtk::Expander::new(Some("Fine-tune detection"));
    let fine_tune_body = column(8);
    fine_tune_body.append(&tolerance_row);
    fine_tune_body.append(&label(
        "Increase this only if normal head movement triggers reminders.",
        "muted",
    ));
    fine_tune.set_child(Some(&fine_tune_body));
    settings_card.append(&fine_tune);
    settings_card.append(&label("Your sound repeats until you look back at a working position. Pause switches the camera off. Closing the window keeps your session in the tray.","muted"));
    left.append(&sound_card);
    right.append(&settings_card);
    body.append(&left);
    body.append(&right);
    root.append(&body);
    let notice = label(
        warning
            .as_deref()
            .unwrap_or(if available.borrow().is_empty() {
                "Connect your webcam, then choose Refresh to find it."
            } else {
                "Start setup, then set the position of each screen."
            }),
        "notice",
    );
    rebuild_sound_sites(&sound_sites, &state, &notice);
    add_site.connect_clicked({
        let (state, notice, list, site_name, site_url, site_editor) = (
            state.clone(),
            notice.clone(),
            sound_sites.clone(),
            site_name.clone(),
            site_url.clone(),
            site_editor.clone(),
        );
        move |_| {
            let result = SoundSite::new(&site_name.text(), &site_url.text()).and_then(|site| {
                let mut state = state.borrow_mut();
                state.settings.add_sound_site(site)?;
                state.save()
            });
            if result.is_ok() {
                site_name.set_text("");
                site_url.set_text("");
                site_editor.set_expanded(false);
                rebuild_sound_sites(&list, &state, &notice);
            }
            feedback(
                &notice,
                result,
                "Website saved. Click its name to open it in your browser.",
            );
        }
    });
    notice.set_selectable(true);
    root.append(&notice);
    root.append(&label("Estimates look direction. It cannot confirm handwriting, scrolling, or mental focus. Unclear readings do not start reminders.","muted"));
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&root)
        .build();
    window.set_child(Some(&scroll));
    background.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });
    start.connect_clicked({let (state,status,notice)=(state.clone(),status.clone(),notice.clone()); move |_| {
        let mut state = state.borrow_mut();
        if state.worker.is_some() { state.pause(); status.set_text("Paused · camera is off"); return; }
        if state.blocked { notice.set_text("Resolve the saved settings error before starting. The original file is preserved."); return; }
        let identity = camera::identity(&state.settings.camera);
        if identity != state.settings.camera_identity {
            state.settings.zones = Default::default(); state.settings.camera_identity = identity;
            notice.set_text("Camera changed. Calibrate your working positions again.");
        }
        match Worker::start(&state.settings.camera,false) {
            Ok(worker) => { state.worker=Some(worker); state.last_result=Instant::now(); status.set_text("Starting camera…"); }
            Err(error) => notice.set_text(&format!("{error:#}")),
        }
    }});
    preview_toggle.connect_toggled({
        let (state, picture, window) = (state.clone(), picture.clone(), window.clone());
        move |toggle| {
            picture.set_visible(toggle.is_active());
            if let Some(worker) = &state.borrow().worker {
                worker
                    .preview
                    .store(toggle.is_active() && window.is_visible(), Ordering::Relaxed);
            }
            if !toggle.is_active() {
                picture.set_paintable(None::<&gdk::Paintable>);
            }
        }
    });
    cameras.connect_selected_notify({
        let (state, available, status, notice) = (
            state.clone(),
            available.clone(),
            status.clone(),
            notice.clone(),
        );
        move |dropdown| {
            let Some((path, _)) = available
                .borrow()
                .get(dropdown.selected() as usize)
                .cloned()
            else {
                return;
            };
            let mut state = state.borrow_mut();
            if state.settings.camera == path {
                return;
            }
            state.pause();
            state.settings.camera = path;
            state.settings.camera_identity.clear();
            state.settings.zones = Default::default();
            status.set_text("Paused · camera changed");
            feedback(
                &notice,
                state.save(),
                "Camera changed. Recalibrate your desk.",
            );
        }
    });
    refresh.connect_clicked({
        let (available, cameras) = (available.clone(), cameras.clone());
        move |_| {
            *available.borrow_mut() = camera::cameras();
            let names: Vec<String> = available
                .borrow()
                .iter()
                .enumerate()
                .map(|(index, (_, name))| {
                    format!(
                        "{} · Camera {}",
                        name.split(" · /dev/").next().unwrap_or(name),
                        index + 1
                    )
                })
                .collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            cameras.set_model(Some(&gtk::StringList::new(&refs)));
        }
    });
    for (i, button) in zone_buttons.iter().enumerate() {
        button.connect_clicked({
            let (state, notice) = (state.clone(), notice.clone());
            move |_| {
                let mut state = state.borrow_mut();
                if state.worker.is_none() {
                    notice.set_text("Start the session before calibrating.");
                    return;
                }
                state.reset_alert();
                state.calibration = Some(Calibration {
                    zone: i,
                    started: Instant::now(),
                    samples: Vec::new(),
                });
                notice.set_text(&format!(
                    "Look toward {}. Hold a natural working position.",
                    NAMES[i]
                ));
            }
        });
    }
    clear.connect_clicked({
        let (state, notice, window) = (state.clone(), notice.clone(), window.clone());
        move |_| {
            let dialog = gtk::AlertDialog::builder()
                .message("Reset your saved positions?")
                .detail("Your sound and settings will stay. Set up both screens again before reminders can resume.")
                .buttons(["Keep positions", "Reset positions"])
                .cancel_button(0)
                .default_button(0)
                .build();
            dialog.choose(Some(&window), None::<&gio::Cancellable>, {
                let (state, notice) = (state.clone(), notice.clone());
                move |choice| {
                    if choice == Ok(1) {
                        let mut state = state.borrow_mut();
                        state.settings.zones = Default::default();
                        state.calibration = None;
                        state.reset_alert();
                        feedback(&notice, state.save(), "Positions reset. Set up both screens before reminders resume.");
                    }
                }
            });
        }
    });
    notes.connect_active_notify({
        let (state, notice, notebook_row) = (state.clone(), notice.clone(), notebook_row.clone());
        move |toggle| {
            if state.borrow().settings.notes == toggle.is_active() {
                return;
            }
            let mut state = state.borrow_mut();
            state.settings.notes = toggle.is_active();
            if !toggle.is_active()
                && state
                    .calibration
                    .as_ref()
                    .is_some_and(|capture| capture.zone == 2)
            {
                state.calibration = None;
            }
            state.reset_alert();
            notebook_row.set_visible(toggle.is_active());
            let message = if !toggle.is_active() {
                "Notebook mode off. Calibration is saved for next time."
            } else if state.settings.zones[2].is_some() {
                "Notebook mode on. Your saved calibration is ready."
            } else {
                "Notebook mode on. Set its position before taking notes."
            };
            feedback(&notice, state.save(), message);
        }
    });
    delay.connect_value_changed({
        let (state, notice) = (state.clone(), notice.clone());
        move |value| {
            let mut state = state.borrow_mut();
            state.settings.delay = value.value_as_int() as u64;
            state.reset_alert();
            feedback(&notice, state.save(), "Alert delay saved.");
        }
    });
    tolerance.connect_value_changed({
        let (state, notice) = (state.clone(), notice.clone());
        move |value| {
            let mut state = state.borrow_mut();
            state.settings.tolerance = value.value();
            state.reset_alert();
            feedback(&notice, state.save(), "Position tolerance saved.");
        }
    });
    choose.connect_clicked({
        let (state, notice, window, sound_name) = (
            state.clone(),
            notice.clone(),
            window.clone(),
            sound_name.clone(),
        );
        move |_| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose your reminder sound")
                .modal(true)
                .build();
            let folder = state
                .borrow()
                .settings
                .sound
                .as_ref()
                .and_then(|path| path.parent().map(PathBuf::from))
                .or_else(dirs::download_dir);
            if let Some(folder) = folder.filter(|path| path.is_dir()) {
                dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
            }
            dialog.open(Some(&window), None::<&gio::Cancellable>, {
                let (state, notice, sound_name) =
                    (state.clone(), notice.clone(), sound_name.clone());
                move |result| {
                    if let Ok(file) = result {
                        let Some(path) = file.path().filter(|path| path.is_file()) else {
                            notice.set_text("Choose a local audio file.");
                            return;
                        };
                        sound_name
                            .set_text(&path.file_name().unwrap_or_default().to_string_lossy());
                        let mut state = state.borrow_mut();
                        state.settings.sound = Some(path);
                        feedback(
                            &notice,
                            state.save(),
                            "Sound saved. Choose Preview sound to hear it.",
                        );
                    }
                }
            });
        }
    });
    test.connect_clicked({
        let (state, notice) = (state.clone(), notice.clone());
        move |_| {
            let mut state = state.borrow_mut();
            let looping = state.gate.active();
            let result = state.sound().and_then(|path| {
                if looping {
                    state.audio.play_loop(&path)
                } else {
                    state.audio.play(&path)
                }
            });
            feedback(&notice, result, "Playing your selected sound.");
        }
    });
    default_sound.connect_clicked({
        let (state, notice, sound_name) = (state.clone(), notice.clone(), sound_name.clone());
        move |_| {
            let mut state = state.borrow_mut();
            state.settings.sound = None;
            sound_name.set_text("Default chime");
            feedback(&notice, state.save(), "Default chime selected.");
        }
    });
    for name in ["show", "hide", "toggle-session", "quit"] {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate({
            let (window, start, app) = (window.clone(), start.clone(), app.clone());
            move |action, _| match action.name().as_str() {
                "show" => window.present(),
                "hide" => window.close(),
                "toggle-session" => start.emit_clicked(),
                "quit" => app.quit(),
                _ => {}
            }
        });
        app.add_action(&action);
    }
    let notebook_action = gio::SimpleAction::new("notebook-mode", Some(glib::VariantTy::BOOLEAN));
    notebook_action.connect_activate({
        let notes = notes.clone();
        move |_, parameter| {
            if let Some(enabled) = parameter.and_then(|value| value.get::<bool>()) {
                notes.set_active(enabled);
            }
        }
    });
    app.add_action(&notebook_action);
    let (tray_icon, tray_commands) = match TrayIcon::start(state.borrow().settings.notes) {
        Ok((tray, commands)) => (Some(tray), Some(commands)),
        Err(error) => {
            notice.set_text(&format!(
                "Tray unavailable: {error}. Closing the window will quit."
            ));
            (None, None)
        }
    };
    let tray_icon = Rc::new(RefCell::new(tray_icon));
    let timer = glib::timeout_add_local(Duration::from_millis(100), {
        let (
            state,
            status,
            notice,
            progress,
            picture,
            start,
            zone_buttons,
            metrics,
            preview_toggle,
            notes,
            notebook_row,
            window,
            app,
            tray_icon,
            background,
            setup_hint,
        ) = (
            state.clone(),
            status.clone(),
            notice.clone(),
            progress.clone(),
            picture.clone(),
            start.clone(),
            zone_buttons.clone(),
            metrics.clone(),
            preview_toggle.clone(),
            notes.clone(),
            notebook_row.clone(),
            window.clone(),
            app.clone(),
            tray_icon.clone(),
            background.clone(),
            setup_hint.clone(),
        );
        move || {
            if let Some(commands) = &tray_commands {
                for command in commands.try_iter() {
                    match command {
                        Command::Open => app.activate_action("show", None),
                        Command::ToggleSession => app.activate_action("toggle-session", None),
                        Command::Notebook(enabled) => {
                            app.activate_action("notebook-mode", Some(&enabled.to_variant()))
                        }
                        Command::Quit => {
                            app.quit();
                            return glib::ControlFlow::Continue;
                        }
                    }
                }
            }
            let mut state = state.borrow_mut();
            if let Err(error) = state.audio.poll() {
                notice.set_text(&format!("{error:#}"));
            }
            if let Some(worker) = &state.worker {
                worker.preview.store(
                    preview_toggle.is_active() && window.is_visible(),
                    Ordering::Relaxed,
                );
            }
            let sample = state.worker.as_ref().and_then(Worker::poll);
            if let Some(sample) = sample {
                match sample {
                    Err(error) => {
                        state.pause();
                        status.set_text("Camera stopped");
                        notice.set_text(&error);
                    }
                    Ok(sample) => {
                        state.last_result = Instant::now();
                        if let Some(frame) = sample.preview {
                            let bytes = glib::Bytes::from_owned(frame.into_raw());
                            let texture = gdk::MemoryTexture::new(
                                640,
                                480,
                                gdk::MemoryFormat::R8g8b8,
                                &bytes,
                                640 * 3,
                            );
                            picture.set_paintable(Some(&texture));
                        }
                        metrics.set_text(&format!(
                            "{:.1} ms inference · {} face(s) · 10 FPS cap",
                            sample.observation.milliseconds, sample.observation.faces
                        ));
                        let features = if sample.captured.elapsed() < Duration::from_millis(750) {
                            sample.observation.features
                        } else {
                            None
                        };
                        if let Some(calibration) = state.calibration.as_mut() {
                            let age = calibration.started.elapsed();
                            if age >= Duration::from_secs(2)
                                && let Some(features) = features
                            {
                                calibration.samples.push(features);
                            }
                        } else {
                            let attention = focus::classify(
                                features.as_ref(),
                                &state.settings.zones,
                                state.settings.notes,
                                state.settings.tolerance,
                            );
                            status.set_text(&match attention {
                                Attention::Working(i) => format!("Working · {}", NAMES[i]),
                                Attention::Phone => "Looking toward phone position".into(),
                                Attention::Distracted => {
                                    "Looking away from your working positions".into()
                                }
                                Attention::Unknown => {
                                    if state.settings.zones[0].is_none()
                                        || state.settings.zones[1].is_none()
                                    {
                                        "Set both screen positions to finish setup".into()
                                    } else {
                                        "Looking for a clear view of your eyes".into()
                                    }
                                }
                            });
                            let now = state.clock.elapsed();
                            let delay = Duration::from_secs(state.settings.delay);
                            let was_alerting = state.gate.active();
                            let alerting = state.gate.update(attention, now, delay);
                            if alerting && !was_alerting {
                                let result =
                                    state.sound().and_then(|path| state.audio.play_loop(&path));
                                feedback(
                                    &notice,
                                    result,
                                    "Sound looping. Look back at a working position to stop it.",
                                );
                            } else if was_alerting && !alerting {
                                state.audio.stop();
                                notice.set_text("Back at work · sound stopped.");
                            }
                        }
                    }
                }
            }
            if let Some(calibration) = &state.calibration {
                let age = calibration.started.elapsed().as_secs_f64();
                status.set_text(&format!(
                    "{} · {}",
                    NAMES[calibration.zone],
                    if age < 2. {
                        format!("look there in {:.0}s", (2. - age).ceil())
                    } else {
                        if age < 6. {
                            format!("keep looking here · {:.0}s left", (6. - age).ceil())
                        } else {
                            "keep your eyes visible · still learning".into()
                        }
                    }
                ));
                progress.set_fraction(((age - 2.) / 4.).clamp(0., 1.));
                if age >= 12. || (age >= 6. && calibration.samples.len() >= 20) {
                    let calibration = state.calibration.take().unwrap();
                    if let Some(zone) = Zone::fit(&calibration.samples) {
                        let overlap = state.settings.zones.iter().enumerate().any(|(i, other)| {
                            i != calibration.zone
                                && (i != 2 || state.settings.notes)
                                && (i == 3 || calibration.zone == 3)
                                && other.as_ref().is_some_and(|other| {
                                    zone.distance(&other.mean).min(other.distance(&zone.mean)) < 3.
                                })
                        });
                        if overlap {
                            notice.set_text("Phone and working positions overlap. Move the phone farther from the notebook/screens, then recalibrate.");
                        } else {
                            state.settings.zones[calibration.zone] = Some(zone);
                            feedback(
                                &notice,
                                state.save(),
                                &format!(
                                    "{} calibrated. Recalibrate after moving your camera or desk.",
                                    NAMES[calibration.zone]
                                ),
                            );
                        }
                    } else {
                        notice.set_text("Your eyes were not clear enough to save this position. Try better lighting, face the camera, and keep your head steady.");
                    }
                    state.reset_alert();
                }
            } else {
                progress.set_fraction(state.gate.progress(
                    state.clock.elapsed(),
                    Duration::from_secs(state.settings.delay),
                ));
            }
            if state.worker.is_some() && state.last_result.elapsed() > Duration::from_secs(2) {
                state.reset_alert();
                status.set_text("Waiting for fresh camera frames…");
            }
            let screens_ready = state.settings.zones[..2].iter().flatten().count();
            setup_hint.set_text(if screens_ready == 2 {
                if state.worker.is_some() {
                    "Your saved positions are active. Pause any time to switch the camera off."
                } else {
                    "Your screen positions are saved. Start a session when you're ready."
                }
            } else if state.worker.is_some() {
                "Setup is in progress. Set both screen positions to enable reminders."
            } else {
                "First time? Choose Start setup, then set the position of each screen."
            });
            start.set_label(if state.worker.is_some() {
                "Pause session"
            } else if screens_ready < 2 {
                "Start setup"
            } else {
                "Start session"
            });
            if state.gate.active() {
                status.add_css_class("alerting");
            } else {
                status.remove_css_class("alerting");
            }
            for (i, button) in zone_buttons.iter().enumerate() {
                button.set_label(if state.settings.zones[i].is_some() {
                    "Update position"
                } else {
                    "Set position"
                });
                button.set_sensitive(state.worker.is_some() && state.calibration.is_none());
            }
            if state.worker.is_none() {
                picture.set_paintable(None::<&gdk::Paintable>);
                progress.set_fraction(0.);
            }
            notebook_row.set_visible(state.settings.notes);
            let tray_available = tray_icon.borrow().as_ref().is_some_and(TrayIcon::available);
            background.set_sensitive(tray_available);
            if let Some(tray) = tray_icon.borrow_mut().as_mut() {
                tray.update(
                    status.text().to_string(),
                    state.worker.is_some(),
                    state.settings.notes,
                );
            }
            if !tray_available && !window.is_visible() {
                window.present();
                notice.set_text("The tray is unavailable. The window has reopened so session controls stay accessible.");
            }
            let allow_notes = state.settings.notes;
            drop(state);
            if notes.is_active() != allow_notes {
                notes.set_active(allow_notes);
            }
            glib::ControlFlow::Continue
        }
    });
    window.connect_close_request({
        let (state, tray_icon, picture) = (state.clone(), tray_icon.clone(), picture.clone());
        move |window| {
            if tray_icon.borrow().as_ref().is_some_and(TrayIcon::available) {
                if let Some(worker) = &state.borrow().worker {
                    worker.preview.store(false, Ordering::Relaxed);
                }
                picture.set_paintable(None::<&gdk::Paintable>);
                window.set_visible(false);
                glib::Propagation::Stop
            } else {
                state.borrow_mut().pause();
                glib::Propagation::Proceed
            }
        }
    });
    let sources = RefCell::new(Some((timer, theme_timer)));
    let shutdown_state = state.clone();
    app.connect_shutdown(move |_| {
        if let Some((timer, theme_timer)) = sources.borrow_mut().take() {
            timer.remove();
            theme_timer.remove();
        }
        shutdown_state.borrow_mut().pause();
        tray_icon.borrow_mut().take();
    });
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed({
        let (state, start, app) = (state.clone(), start.clone(), app.clone());
        move |_, key, _, modifiers| {
            if key == gdk::Key::q && modifiers.contains(gdk::ModifierType::CONTROL_MASK) {
                app.quit();
                return glib::Propagation::Stop;
            }
            if key == gdk::Key::space && state.borrow().worker.is_some() {
                start.emit_clicked();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });
    window.add_controller(keys);
    window.present();
    if smoke {
        let window = window.clone();
        let app = app.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            let paintable = gtk::WidgetPaintable::new(Some(&window));
            let snapshot = gtk::Snapshot::new();
            paintable.snapshot(&snapshot, window.width() as f64, window.height() as f64);
            if let Some(node) = snapshot.to_node()
                && let Some(surface) = window.surface()
                && let Some(renderer) = gtk::gsk::Renderer::for_surface(&surface)
            {
                let texture = renderer.render_texture(&node, None);
                if let Err(error) = texture.save_to_png("target/ui-smoke.png") {
                    eprintln!("Screenshot: {error}");
                }
                renderer.unrealize();
            }
            println!("GUI smoke passed; no webcam opened.");
            app.quit();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accent_text_keeps_contrast_for_light_and_dark_themes() {
        assert_eq!(accent_foreground("#ffffff"), "black");
        assert_eq!(accent_foreground("#000000"), "white");
        assert_eq!(accent_foreground("#a8d5ba"), "black");
    }
}
