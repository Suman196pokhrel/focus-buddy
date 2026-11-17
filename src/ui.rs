use crate::{
    audio::Audio,
    camera::{self, Worker},
    focus::{self, AlertGate, Attention, Features, NAMES, Zone},
    settings::{self, Settings},
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
        self.audio.stop();
        self.calibration = None;
        self.gate.reset();
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
fn theme(provider: &gtk::CssProvider) {
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
    provider.load_from_data(&format!(r#"
window {{ background: #10161b; color: #edf1ef; }}
.root {{ padding: 28px; }} .title {{ font-size: 30px; font-weight: 800; }}
.eyebrow {{ color: {accent}; font-size: 11px; font-weight: 700; letter-spacing: 2px; }}
.muted {{ color: #9faeb5; font-size: 12px; }} .section-title {{ font-size: 15px; font-weight: 700; }}
.status {{ font-size: 20px; font-weight: 700; }}
.card {{ background: #182127; padding: 20px; border: 1px solid #2c393f; border-radius: 0; }}
button {{ padding: 9px 14px; border-radius: 0; }} button.primary {{ background: {accent}; color: #10161b; font-weight: 700; }}
.notice {{ color: #e3c698; font-size: 12px; }} progressbar progress {{ background: {accent}; }}
.preview {{ background: #0c1115; border: 1px solid #2c393f; }}
"#));
}

pub fn launch(smoke: bool) -> glib::ExitCode {
    let app = gtk::Application::builder()
        .application_id("dev.focusbuddy.FocusBuddy")
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
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
    theme(&provider);
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let theme_timer = glib::timeout_add_local(Duration::from_secs(5), move || {
        theme(&provider);
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
        "Your webcam stays local. Your attention gets a gentle reminder.",
        "muted",
    ));
    heading.append(&titles);
    let start = gtk::Button::with_label("Start session");
    start.add_css_class("primary");
    heading.append(&start);
    let background = gtk::Button::with_label("Minimize");
    background.set_tooltip_text(Some("Keep monitoring while the window is minimized"));
    heading.append(&background);
    root.append(&heading);
    let body = row(20);
    let left = column(16);
    left.set_hexpand(true);
    let right = column(16);
    right.set_width_request(400);
    let live = card("01   SESSION");
    let status = label("Paused · camera released", "status");
    live.append(&status);
    let progress = gtk::ProgressBar::new();
    live.append(&progress);
    let metrics = label(
        "10 FPS analysis cap · one inference thread per model",
        "muted",
    );
    live.append(&metrics);
    left.append(&live);
    let preview_card = card("02   CAMERA");
    let available = Rc::new(RefCell::new(camera::cameras()));
    let names: Vec<String> = available
        .borrow()
        .iter()
        .map(|(_, name)| name.clone())
        .collect();
    let names_ref: Vec<&str> = names.iter().map(String::as_str).collect();
    let cameras = gtk::DropDown::from_strings(&names_ref);
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
    preview_card.append(&picture);
    let preview_toggle = gtk::CheckButton::with_label("Show camera preview");
    preview_card.append(&preview_toggle);
    preview_card.append(&label(
        "Preview is off by default. Pausing releases the camera.",
        "muted",
    ));
    left.append(&preview_card);
    let calibration = card("03   LEARN YOUR DESK");
    calibration.append(&label("Start the camera, then calibrate each position. You get 2 seconds to look at the target, followed by 4 seconds of samples.","muted"));
    let mut zone_buttons = Vec::new();
    for (i, name) in NAMES.iter().enumerate() {
        let zone_row = row(10);
        let text = label(name, "section-title");
        text.set_hexpand(true);
        zone_row.append(&text);
        let button = gtk::Button::with_label(if state.borrow().settings.zones[i].is_some() {
            "Recalibrate"
        } else {
            "Calibrate"
        });
        zone_row.append(&button);
        calibration.append(&zone_row);
        zone_buttons.push(button);
    }
    calibration.append(&label("Both monitors are required. Notebook and phone are optional. Keep your phone in a different place from your notebook.","muted"));
    let notes = gtk::CheckButton::with_label("Allow looking at my notebook");
    notes.set_active(state.borrow().settings.notes);
    calibration.append(&notes);
    let clear = gtk::Button::with_label("Clear calibration");
    calibration.append(&clear);
    right.append(&calibration);
    let sound_card = card("04   YOUR NUDGE");
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
    let test = gtk::Button::with_label("Test");
    let default_sound = gtk::Button::with_label("Default");
    sound_row.append(&choose);
    sound_row.append(&test);
    sound_row.append(&default_sound);
    sound_card.append(&sound_row);
    let timing = row(8);
    let timing_label = label("Alert after (seconds)", "muted");
    timing_label.set_hexpand(true);
    timing.append(&timing_label);
    let delay = gtk::SpinButton::with_range(2., 30., 1.);
    delay.set_value(state.borrow().settings.delay as f64);
    timing.append(&delay);
    sound_card.append(&timing);
    let tolerance_row = row(8);
    let tolerance_label = label("Position tolerance", "muted");
    tolerance_label.set_hexpand(true);
    tolerance_row.append(&tolerance_label);
    let tolerance = gtk::SpinButton::with_range(1.5, 4., 0.1);
    tolerance.set_digits(1);
    tolerance.set_value(state.borrow().settings.tolerance);
    tolerance_row.append(&tolerance);
    sound_card.append(&tolerance_row);
    sound_card.append(&label(
        "Higher tolerance allows more movement. Alerts have a 25-second cooldown.",
        "muted",
    ));
    right.append(&sound_card);
    body.append(&left);
    body.append(&right);
    root.append(&body);
    let notice = label(
        warning
            .as_deref()
            .unwrap_or(if available.borrow().is_empty() {
                "No webcam detected. Connect one, then press Refresh."
            } else {
                "Calibrate both monitors to enable reminders."
            }),
        "notice",
    );
    notice.set_selectable(true);
    root.append(&notice);
    root.append(&label("Estimates look direction. It cannot confirm handwriting, scrolling, or mental focus. Uncertain readings stay quiet.","muted"));
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&root)
        .build();
    window.set_child(Some(&scroll));
    background.connect_clicked({
        let window = window.clone();
        move |_| window.minimize()
    });
    start.connect_clicked({let (state,status,notice)=(state.clone(),status.clone(),notice.clone()); move |_| {
        let mut state = state.borrow_mut();
        if state.worker.is_some() { state.pause(); status.set_text("Paused · camera released"); return; }
        if state.blocked { notice.set_text("Resolve the saved settings error before starting. The original file is preserved."); return; }
        let identity = camera::identity(&state.settings.camera);
        if identity != state.settings.camera_identity {
            state.settings.zones = Default::default(); state.settings.notes = false; state.settings.camera_identity = identity;
            notice.set_text("Camera changed. Calibrate your working positions again.");
        }
        match Worker::start(&state.settings.camera,false) {
            Ok(worker) => { state.worker=Some(worker); state.last_result=Instant::now(); status.set_text("Starting camera…"); }
            Err(error) => notice.set_text(&format!("{error:#}")),
        }
    }});
    preview_toggle.connect_toggled({
        let (state, picture) = (state.clone(), picture.clone());
        move |toggle| {
            if let Some(worker) = &state.borrow().worker {
                worker.preview.store(toggle.is_active(), Ordering::Relaxed);
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
            state.settings.notes = false;
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
                .map(|(_, name)| name.clone())
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
                state.gate.reset();
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
        let (state, notice) = (state.clone(), notice.clone());
        move |_| {
            let mut state = state.borrow_mut();
            state.settings.zones = Default::default();
            state.settings.notes = false;
            state.calibration = None;
            state.gate.reset();
            feedback(
                &notice,
                state.save(),
                "Calibration cleared. Recalibrate both monitors before reminders resume.",
            );
        }
    });
    notes.connect_toggled({
        let (state, notice) = (state.clone(), notice.clone());
        move |toggle| {
            if state.borrow().settings.notes == toggle.is_active() {
                return;
            }
            if toggle.is_active() && state.borrow().settings.zones[2].is_none() {
                notice.set_text("Calibrate the notebook first.");
                toggle.set_active(false);
                return;
            }
            let mut state = state.borrow_mut();
            state.settings.notes = toggle.is_active();
            state.gate.reset();
            feedback(&notice, state.save(), "Notebook preference saved.");
        }
    });
    delay.connect_value_changed({
        let (state, notice) = (state.clone(), notice.clone());
        move |value| {
            let mut state = state.borrow_mut();
            state.settings.delay = value.value_as_int() as u64;
            state.gate.reset();
            feedback(&notice, state.save(), "Alert delay saved.");
        }
    });
    tolerance.connect_value_changed({
        let (state, notice) = (state.clone(), notice.clone());
        move |value| {
            let mut state = state.borrow_mut();
            state.settings.tolerance = value.value();
            state.gate.reset();
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
                            "Sound saved. Press Test to check playback.",
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
            let result = state.sound().and_then(|path| state.audio.play(&path));
            feedback(&notice, result, "Playing your reminder sound.");
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
        );
        move || {
            let mut state = state.borrow_mut();
            if let Err(error) = state.audio.poll() {
                notice.set_text(&format!("{error:#}"));
            }
            if let Some(worker) = &state.worker {
                worker
                    .preview
                    .store(preview_toggle.is_active(), Ordering::Relaxed);
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
                                Attention::Distracted => "Looking outside working positions".into(),
                                Attention::Unknown => {
                                    if state.settings.zones[0].is_none()
                                        || state.settings.zones[1].is_none()
                                    {
                                        "Calibrate both monitors to begin".into()
                                    } else {
                                        "Uncertain · waiting quietly".into()
                                    }
                                }
                            });
                            let now = state.clock.elapsed();
                            let delay = Duration::from_secs(state.settings.delay);
                            let cooldown = Duration::from_secs(state.settings.cooldown);
                            if state.gate.update(attention, now, delay, cooldown) {
                                let result = state.sound().and_then(|path| state.audio.play(&path));
                                feedback(
                                    &notice,
                                    result,
                                    "A little nudge. Come back when you’re ready.",
                                );
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
                        format!("collecting · {} samples", calibration.samples.len())
                    }
                ));
                progress.set_fraction(((age - 2.) / 4.).clamp(0., 1.));
                if age >= 12. || (age >= 6. && calibration.samples.len() >= 20) {
                    let calibration = state.calibration.take().unwrap();
                    if let Some(zone) = Zone::fit(&calibration.samples) {
                        let overlap = state.settings.zones.iter().enumerate().any(|(i, other)| {
                            i != calibration.zone
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
                        notice.set_text("Could not get stable eye readings. Improve lighting, face the camera, and retry. Glasses or looking too far down can prevent calibration.");
                    }
                    state.gate.reset();
                }
            } else {
                progress.set_fraction(state.gate.progress(
                    state.clock.elapsed(),
                    Duration::from_secs(state.settings.delay),
                ));
            }
            if state.worker.is_some() && state.last_result.elapsed() > Duration::from_secs(2) {
                state.gate.reset();
                status.set_text("Waiting for fresh camera frames…");
            }
            start.set_label(if state.worker.is_some() {
                "Pause session"
            } else {
                "Start session"
            });
            for (i, button) in zone_buttons.iter().enumerate() {
                button.set_label(if state.settings.zones[i].is_some() {
                    "Recalibrate"
                } else {
                    "Calibrate"
                });
                button.set_sensitive(state.calibration.is_none());
            }
            if state.worker.is_none() {
                picture.set_paintable(None::<&gdk::Paintable>);
                progress.set_fraction(0.);
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
        let state = state.clone();
        move |_| {
            state.borrow_mut().pause();
            glib::Propagation::Proceed
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
