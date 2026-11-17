use ksni::{
    Tray,
    blocking::TrayMethods,
    menu::{CheckmarkItem, StandardItem},
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, SyncSender},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Open,
    ToggleSession,
    Notebook(bool),
    Quit,
}

pub struct FocusTray {
    sender: SyncSender<Command>,
    online: Arc<AtomicBool>,
    pub summary: String,
    pub running: bool,
    pub notebook: bool,
}
impl FocusTray {
    fn send(&self, command: Command) {
        let _ = self.sender.try_send(command);
    }
}
impl Tray for FocusTray {
    fn id(&self) -> String {
        "focus-buddy".into()
    }
    fn icon_name(&self) -> String {
        "focus-buddy".into()
    }
    fn title(&self) -> String {
        "Focus Buddy".into()
    }
    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }
    fn activate(&mut self, _: i32, _: i32) {
        self.send(Command::Open);
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        // Omarchy displays the tooltip title, so keep the useful status there.
        ksni::ToolTip {
            title: format!(
                "Focus Buddy · {} · Notebook {}",
                self.summary,
                if self.notebook { "on" } else { "off" }
            ),
            description: "Click to open. Right-click for session controls.".into(),
            ..Default::default()
        }
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            StandardItem {
                label: self.summary.clone(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Open Focus Buddy".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Command::Open)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: if self.running {
                    "Pause session"
                } else {
                    "Start session"
                }
                .into(),
                activate: Box::new(|tray: &mut Self| tray.send(Command::ToggleSession)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Notebook mode".into(),
                checked: self.notebook,
                activate: Box::new(|tray: &mut Self| tray.send(Command::Notebook(!tray.notebook))),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit Focus Buddy".into(),
                activate: Box::new(|tray: &mut Self| tray.send(Command::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
    fn watcher_online(&self) {
        self.online.store(true, Ordering::Relaxed);
    }
    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.online.store(false, Ordering::Relaxed);
        self.send(Command::Open);
        true
    }
}
pub struct TrayIcon {
    handle: ksni::blocking::Handle<FocusTray>,
    online: Arc<AtomicBool>,
    last: (String, bool, bool),
}
impl TrayIcon {
    pub fn start(notebook: bool) -> Result<(Self, Receiver<Command>), ksni::Error> {
        let (sender, receiver) = mpsc::sync_channel(16);
        let online = Arc::new(AtomicBool::new(true));
        let summary = "Paused · camera released".to_string();
        let handle = FocusTray {
            sender,
            online: online.clone(),
            summary: summary.clone(),
            running: false,
            notebook,
        }
        .spawn()?;
        Ok((
            Self {
                handle,
                online,
                last: (summary, false, notebook),
            },
            receiver,
        ))
    }
    pub fn available(&self) -> bool {
        self.online.load(Ordering::Relaxed) && !self.handle.is_closed()
    }
    pub fn update(&mut self, summary: String, running: bool, notebook: bool) {
        if self.last == (summary.clone(), running, notebook) {
            return;
        }
        self.last = (summary.clone(), running, notebook);
        self.handle.update(move |tray| {
            tray.summary = summary;
            tray.running = running;
            tray.notebook = notebook;
        });
    }
}
impl Drop for TrayIcon {
    fn drop(&mut self) {
        let _ = self.handle.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_commands_follow_the_current_session_state() {
        let (sender, receiver) = mpsc::sync_channel(16);
        let mut tray = FocusTray {
            sender,
            online: Arc::new(AtomicBool::new(true)),
            summary: "Paused".into(),
            running: false,
            notebook: false,
        };
        let menu = tray.menu();
        if let ksni::MenuItem::Standard(item) = &menu[3] {
            assert_eq!(item.label, "Start session");
            (item.activate)(&mut tray);
        } else {
            panic!("missing session action");
        }
        assert_eq!(receiver.try_recv().unwrap(), Command::ToggleSession);
        if let ksni::MenuItem::Checkmark(item) = &menu[4] {
            assert!(!item.checked);
            (item.activate)(&mut tray);
        } else {
            panic!("missing notebook action");
        }
        assert_eq!(receiver.try_recv().unwrap(), Command::Notebook(true));
        if let ksni::MenuItem::Standard(item) = &menu[6] {
            (item.activate)(&mut tray);
        } else {
            panic!("missing quit action");
        }
        assert_eq!(receiver.try_recv().unwrap(), Command::Quit);
        tray.running = true;
        if let ksni::MenuItem::Standard(item) = &tray.menu()[3] {
            assert_eq!(item.label, "Pause session");
        }
    }
    #[test]
    fn click_and_tooltip_report_the_same_state() {
        let (sender, receiver) = mpsc::sync_channel(16);
        let mut tray = FocusTray {
            sender,
            online: Arc::new(AtomicBool::new(true)),
            summary: "Working · Monitor 1".into(),
            running: true,
            notebook: false,
        };
        tray.activate(0, 0);
        assert_eq!(receiver.try_recv().unwrap(), Command::Open);
        assert!(tray.tool_tip().title.contains("Working · Monitor 1"));
        assert!(tray.tool_tip().title.contains("Notebook off"));
        tray.watcher_offline(ksni::OfflineReason::No);
        assert!(!tray.online.load(Ordering::Relaxed));
        assert_eq!(receiver.try_recv().unwrap(), Command::Open);
        tray.watcher_online();
        assert!(tray.online.load(Ordering::Relaxed));
    }
}
