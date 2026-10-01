//! Portal Workspaces: terminal workspaces for agentic development.

mod app;
mod fonts;
mod icons;
mod inbox;
mod keymap;
mod persist;
mod sessions;
mod theme;
mod ui;
mod usage;
mod workspace;

use pw_model::store::{self, LoadOutcome};

use crate::app::App;
use crate::persist::Saver;

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

    let saver = std::sync::Arc::new(std::sync::Mutex::new(Some(Saver::new(path))));

    // A daemon rather than an application: terminals can be detached into their own windows, and
    // the app opens its windows itself (see `app::windows`).
    let mut app = iced::daemon(
        move || App::boot(state.clone(), saver.lock().expect("saver").take().expect("boot runs once")),
        App::update,
        App::view,
    )
    .title(App::title)
    .subscription(App::subscription)
    .theme(|_: &App, _| theme::theme())
    .style(|_: &App, t: &iced::Theme| theme::app_style(t))
    .default_font(fonts::UI)
    .antialiasing(true);
    for font in fonts::DATA {
        app = app.font(font);
    }
    app.run()
}
