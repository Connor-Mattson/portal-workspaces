//! Portal Workspaces: terminal workspaces for agentic development.

mod app;
mod fonts;
mod icons;
mod keymap;
mod persist;
mod sessions;
mod theme;
mod ui;
mod workspace;

use iced::{Size, window};
use pw_model::store::{self, LoadOutcome};

use crate::app::App;
use crate::persist::Saver;

/// 64×64 RGBA window icon (generated from `assets/icons/portal-workspaces.png`).
const ICON_RGBA: &[u8] = include_bytes!("../../../assets/icons/portal-workspaces-64.rgba");

fn main() -> iced::Result {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,pw_app=info".into()),
        )
        .init();

    let path = persist::state_path();
    let state = match store::load(&path) {
        LoadOutcome::Recovered { backup, reason } => {
            tracing::warn!(%reason, backup = %backup.display(), "previous state could not be read");
            Default::default()
        }
        outcome => outcome.into_state(),
    };
    tracing::info!(path = %path.display(), workspaces = state.workspaces.len(), "starting");

    let size = state.ui.window.map_or(Size::new(1440.0, 900.0), |w| Size::new(w.width, w.height));
    let saver = std::sync::Arc::new(std::sync::Mutex::new(Some(Saver::new(path))));

    let mut app = iced::application(
        move || App::boot(state.clone(), saver.lock().expect("saver").take().expect("boot runs once")),
        App::update,
        App::view,
    )
    .title(App::title)
    .subscription(App::subscription)
    .theme(|_: &App| theme::theme())
    .style(|_: &App, t: &iced::Theme| theme::app_style(t))
    .default_font(fonts::UI)
    .antialiasing(true)
    .exit_on_close_request(false)
    .window(window::Settings {
        size,
        min_size: Some(Size::new(720.0, 440.0)),
        icon: window::icon::from_rgba(ICON_RGBA.to_vec(), 64, 64).ok(),
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: "portal-workspaces".to_owned(),
            ..Default::default()
        },
        ..Default::default()
    });
    for font in fonts::DATA {
        app = app.font(font);
    }
    app.run()
}
