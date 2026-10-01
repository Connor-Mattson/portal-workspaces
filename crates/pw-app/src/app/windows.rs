//! Windows: the main one, and one per detached terminal.
//!
//! The app is an iced daemon, so it opens its windows itself. A detached terminal leaves its workspace's
//! grid and shows in its own window, with the same session. Its window stays open while you switch
//! workspaces, and closing it docks the terminal back into the grid. Keys go to the window that last had
//! focus: a detached window's terminal, or the main window's focused pane.

use iced::{Size, Task, window};
use pw_model::{PaneId, WindowGeometry};

use crate::app::{App, Message};

/// 64×64 RGBA window icon (generated from `assets/icons/portal-workspaces.png`).
const ICON_RGBA: &[u8] = include_bytes!("../../../../assets/icons/portal-workspaces-64.rgba");

#[derive(Debug, Clone)]
pub enum WindowMsg {
    Focused(window::Id),
    Unfocused(window::Id),
    Resized(window::Id, Size),
    CloseRequested(window::Id),
    Closed(window::Id),
}

pub fn main_settings(window: Option<WindowGeometry>) -> window::Settings {
    let size = window.map_or(Size::new(1440.0, 900.0), |w| Size::new(w.width, w.height));
    settings(size, Size::new(720.0, 440.0))
}

fn terminal_settings(window: Option<WindowGeometry>) -> window::Settings {
    let size = window.map_or(Size::new(900.0, 600.0), |w| Size::new(w.width, w.height));
    settings(size, Size::new(360.0, 220.0))
}

fn settings(size: Size, min_size: Size) -> window::Settings {
    window::Settings {
        size,
        min_size: Some(min_size),
        icon: window::icon::from_rgba(ICON_RGBA.to_vec(), 64, 64).ok(),
        // Closing saves and quits (the main window) or docks the terminal (its own window).
        exit_on_close_request: false,
        // A scripted run stays on top, so its screenshots show what it drew.
        level: if crate::devtools::scripted() { window::Level::AlwaysOnTop } else { window::Level::Normal },
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: "portal-workspaces".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl App {
    pub(super) fn on_window(&mut self, msg: WindowMsg) -> Task<Message> {
        match msg {
            WindowMsg::Focused(id) => {
                if id == self.main_window || self.pane_windows.contains_key(&id) {
                    self.focused_window = Some(id);
                    self.refocus(|app| app.key_window = id);
                }
            }
            // Moving between our own windows reports the new one's focus after this.
            WindowMsg::Unfocused(id) => {
                if self.focused_window == Some(id) {
                    self.focused_window = None;
                }
                // In case the button was let go where we didn't hear it.
                self.settle_terminals();
            }
            WindowMsg::Resized(id, size) => {
                let geometry = Some(WindowGeometry { width: size.width, height: size.height });
                if id == self.main_window {
                    self.prefs.window = geometry;
                } else if self.pane_windows.contains_key(&id) {
                    self.prefs.terminal_window = geometry;
                } else {
                    return Task::none();
                }
                self.touch();
            }
            WindowMsg::CloseRequested(id) if id == self.main_window => return self.request_quit(),
            WindowMsg::CloseRequested(id) => {
                if let Some(&pane) = self.pane_windows.get(&id) {
                    return self.dock(pane);
                }
            }
            // Closed by the system rather than by us: dock, so the terminal isn't lost.
            WindowMsg::Closed(id) => {
                if let Some(&pane) = self.pane_windows.get(&id) {
                    self.forget_window(id);
                    self.dock_into_grid(pane);
                }
            }
        }
        Task::none()
    }

    /// The window a detached pane shows in.
    pub(crate) fn window_of(&self, pane: PaneId) -> Option<window::Id> {
        self.pane_windows.iter().find(|(_, p)| **p == pane).map(|(id, _)| *id)
    }

    /// Moves a pane of the active workspace's grid into its own window. Keys follow when the window
    /// reports it has focus.
    pub(super) fn detach(&mut self, pane: PaneId) -> Task<Message> {
        let mut detached = false;
        self.refocus(|app| detached = app.active_mut().is_some_and(|ws| ws.detach(pane)));
        if !detached {
            return Task::none();
        }
        // The grid's other panes changed size.
        self.sessions.clear_all_caches();
        self.touch();
        self.open_window(pane)
    }

    pub(super) fn open_window(&mut self, pane: PaneId) -> Task<Message> {
        let (id, opened) = window::open(terminal_settings(self.prefs.terminal_window));
        self.pane_windows.insert(id, pane);
        opened.discard()
    }

    /// Closes a detached pane's window and puts the pane back into its workspace's grid.
    pub(super) fn dock(&mut self, pane: PaneId) -> Task<Message> {
        let Some(id) = self.window_of(pane) else { return Task::none() };
        // Docking from the terminal's window continues in the main window.
        let raise = if self.key_window == id { window::gain_focus(self.main_window) } else { Task::none() };
        self.forget_window(id);
        self.dock_into_grid(pane);
        Task::batch([window::close(id), raise])
    }

    fn dock_into_grid(&mut self, pane: PaneId) {
        self.refocus(|app| {
            if let Some(ws) = app.workspaces.iter_mut().find(|ws| ws.is_detached(pane)) {
                ws.dock(pane);
            }
        });
        self.sessions.clear_all_caches();
        self.touch();
    }

    /// Drops a detached window from the registry; keys go back to the main window if they were its.
    pub(super) fn forget_window(&mut self, id: window::Id) {
        self.refocus(|app| {
            app.pane_windows.remove(&id);
            if app.key_window == id {
                app.key_window = app.main_window;
            }
        });
    }

    /// Brings the main window forward when a detached terminal's window has the keys, e.g. for a sheet
    /// opened by a shortcut typed there.
    pub(super) fn raise_main(&self) -> Task<Message> {
        if self.key_window == self.main_window { Task::none() } else { window::gain_focus(self.main_window) }
    }
}
