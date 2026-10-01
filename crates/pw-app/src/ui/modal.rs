//! Sheets shown over the app: workspace editor, confirmations and the shortcut list.

use std::path::PathBuf;

use iced::widget::{Space, button, center, column, container, mouse_area, opaque, row, text, text_input};
use iced::{Alignment, Element, Fill, Length, Task};
use pw_model::{Preset, Workspace, WorkspaceId};

use crate::app::{App, Message};
use crate::fonts;
use crate::keymap;
use crate::theme;
use crate::ui::profile_editor::{self, ProfileEditor, ProfileMsg};
use crate::ui::{preset_glyph, tildify};

const NAME_INPUT: &str = "workspace-name";

pub enum Modal {
    Editor(Editor),
    ConfirmDelete(WorkspaceId),
    ConfirmPreset {
        workspace: WorkspaceId,
        preset: Preset,
        closing: usize,
    },
    Shortcuts,
    /// Track a new AI account, or edit one.
    Profile(ProfileEditor),
}

#[derive(Debug, Clone)]
pub enum ModalMsg {
    Name(String),
    Root(String),
    Browse,
    Browsed(Option<PathBuf>),
    Preset(Preset),
    Profile(ProfileMsg),
    Delete,
    Submit,
    Cancel,
}

/// Create or edit a workspace.
pub struct Editor {
    /// `None` when creating.
    pub target: Option<WorkspaceId>,
    pub name: String,
    pub root: String,
    pub preset: Preset,
    pub error: Option<String>,
    /// The name was typed by hand, so picking a folder shouldn't overwrite it.
    name_edited: bool,
}

impl Editor {
    pub fn new_workspace() -> Self {
        Self {
            target: None,
            name: String::new(),
            root: String::new(),
            preset: Preset::Columns(2),
            error: None,
            name_edited: false,
        }
    }

    pub fn edit(ws: &Workspace) -> Self {
        Self {
            target: Some(ws.id),
            name: ws.name.clone(),
            root: tildify(&ws.root),
            preset: Preset::Single,
            error: None,
            name_edited: true,
        }
    }

    pub fn set_root(&mut self, path: PathBuf) {
        if !self.name_edited || self.name.trim().is_empty() {
            self.name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        }
        self.root = tildify(&path);
        self.error = None;
    }

    /// The root with `~` expanded.
    pub fn resolved_root(&self) -> Option<PathBuf> {
        let raw = self.root.trim();
        if raw.is_empty() {
            return None;
        }
        Some(match raw.strip_prefix('~') {
            Some(rest) => crate::sessions::home_dir().join(rest.trim_start_matches('/')),
            None => PathBuf::from(raw),
        })
    }

    /// The name and an existing directory, or a message saying what's wrong.
    pub fn validate(&self) -> Result<(String, PathBuf), String> {
        let root = self.resolved_root().ok_or("Choose the project folder.")?;
        if !root.is_dir() {
            return Err(format!("{} isn't a folder.", root.display()));
        }
        let root = root.canonicalize().unwrap_or(root);
        let name = match self.name.trim() {
            "" => root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Workspace".into()),
            name => name.to_owned(),
        };
        Ok((name, root))
    }
}

pub fn focus_first_field() -> Task<Message> {
    iced::widget::operation::focus(NAME_INPUT)
}

pub fn view<'a>(app: &'a App, modal: &'a Modal) -> Element<'a, Message> {
    let (width, content) = match modal {
        Modal::Editor(editor) => (460.0, editor_view(app, editor)),
        Modal::ConfirmDelete(id) => {
            let name = app.workspaces.iter().find(|w| w.id() == *id).map_or("this workspace", |w| &w.model.name);
            let n = app.workspaces.iter().find(|w| w.id() == *id).map_or(0, |w| w.pane_count());
            (
                400.0,
                confirm(
                    format!("Delete “{name}”?"),
                    format!(
                        "Its {n} terminal{} will be closed. Files in the project folder are not touched.",
                        if n == 1 { "" } else { "s" }
                    ),
                    "Delete workspace",
                ),
            )
        }
        Modal::ConfirmPreset { closing, .. } => (
            400.0,
            confirm(
                "Close running terminals?".to_owned(),
                format!(
                    "This layout has fewer panes, so the last {closing} terminal{} will be closed.",
                    if *closing == 1 { "" } else { "s" }
                ),
                "Change layout",
            ),
        ),
        Modal::Shortcuts => (440.0, shortcuts()),
        Modal::Profile(editor) => (480.0, profile_editor::view(editor)),
    };
    let card = container(content).width(width).padding(theme::S6).style(theme::card);
    // Clicking the scrim dismisses; `opaque` keeps clicks on the card from reaching it.
    mouse_area(center(opaque(card)).style(theme::scrim)).on_press(Message::Modal(ModalMsg::Cancel)).into()
}

fn field_label(label: &str) -> Element<'_, Message> {
    text(label).size(theme::T_SM).font(fonts::UI_MEDIUM).color(theme::FG_2).into()
}

fn editor_view<'a>(app: &'a App, editor: &'a Editor) -> Element<'a, Message> {
    let creating = editor.target.is_none();
    let first_run = app.workspaces.is_empty();
    let title = if creating { "New workspace" } else { "Edit workspace" };
    let subtitle = if first_run {
        "Welcome to Portal Workspaces. Start by pointing a workspace at a project folder."
    } else if creating {
        "One project folder and the terminals you keep open for it."
    } else {
        "New terminals open in the project folder. Running terminals are not affected."
    };

    let name = text_input("my-project", &editor.name)
        .id(NAME_INPUT)
        .on_input(|s| Message::Modal(ModalMsg::Name(s)))
        .on_submit(Message::Modal(ModalMsg::Submit))
        .padding([8, 10])
        .size(theme::T_MD)
        .font(fonts::UI)
        .style(theme::input);

    let root = row![
        text_input("~/projects/my-project", &editor.root)
            .on_input(|s| Message::Modal(ModalMsg::Root(s)))
            .on_submit(Message::Modal(ModalMsg::Submit))
            .padding([8, 10])
            .size(theme::T_MD)
            .font(fonts::MONO)
            .style(theme::input),
        button(text("Browse…").size(theme::T_MD).font(fonts::UI_MEDIUM))
            .padding([8, 12])
            .style(theme::secondary_button)
            .on_press(Message::Modal(ModalMsg::Browse)),
    ]
    .spacing(theme::S2);

    let mut fields = column![
        column![field_label("Name"), name].spacing(6),
        column![field_label("Project folder"), root].spacing(6),
    ]
    .spacing(theme::S4);

    if creating {
        let presets = row(Preset::ALL.into_iter().map(|preset| {
            let active = editor.preset == preset;
            button(container(preset_glyph(preset, if active { theme::ACCENT } else { theme::FG_3 })).center_x(Fill))
                .width(Length::Fill)
                .padding([7, 0])
                .style(theme::chip(active))
                .on_press(Message::Modal(ModalMsg::Preset(preset)))
                .into()
        }))
        .spacing(theme::S1);
        fields = fields.push(
            column![
                field_label("Terminals"),
                presets,
                text(editor.preset.label()).size(theme::T_XS).font(fonts::UI).color(theme::FG_3),
            ]
            .spacing(6),
        );
    }

    if let Some(error) = &editor.error {
        fields = fields.push(
            container(text(error).size(theme::T_SM).font(fonts::UI))
                .padding([6, 10])
                .width(Fill)
                .style(theme::error_note),
        );
    }

    let mut actions = row![].spacing(theme::S2).align_y(Alignment::Center);
    if !creating {
        actions = actions.push(
            button(text("Delete").size(theme::T_MD).font(fonts::UI_MEDIUM))
                .padding([7, 12])
                .style(theme::ghost_button)
                .on_press(Message::Modal(ModalMsg::Delete)),
        );
    }
    actions = actions.push(Space::new().width(Fill));
    if !first_run {
        actions = actions.push(
            button(text("Cancel").size(theme::T_MD).font(fonts::UI_MEDIUM))
                .padding([7, 14])
                .style(theme::secondary_button)
                .on_press(Message::Modal(ModalMsg::Cancel)),
        );
    }
    actions = actions.push(
        button(text(if creating { "Create workspace" } else { "Save" }).size(theme::T_MD).font(fonts::UI_SEMIBOLD))
            .padding([7, 14])
            .style(theme::primary_button)
            .on_press(Message::Modal(ModalMsg::Submit)),
    );

    column![
        column![
            text(title).size(18).font(fonts::UI_SEMIBOLD).color(theme::FG),
            text(subtitle).size(theme::T_MD).font(fonts::UI).color(theme::FG_3),
        ]
        .spacing(6),
        fields,
        actions,
    ]
    .spacing(theme::S6)
    .into()
}

fn confirm<'a>(title: String, detail: String, action: &'a str) -> Element<'a, Message> {
    column![
        text(title).size(18).font(fonts::UI_SEMIBOLD).color(theme::FG),
        text(detail).size(theme::T_MD).font(fonts::UI).color(theme::FG_2),
        row![
            Space::new().width(Fill),
            button(text("Cancel").size(theme::T_MD).font(fonts::UI_MEDIUM))
                .padding([7, 14])
                .style(theme::secondary_button)
                .on_press(Message::Modal(ModalMsg::Cancel)),
            button(text(action).size(theme::T_MD).font(fonts::UI_SEMIBOLD))
                .padding([7, 14])
                .style(theme::danger_button)
                .on_press(Message::Modal(ModalMsg::Submit)),
        ]
        .spacing(theme::S2),
    ]
    .spacing(theme::S4)
    .into()
}

fn shortcuts<'a>() -> Element<'a, Message> {
    let rows = column(keymap::cheatsheet().into_iter().map(|(label, keys)| {
        row![
            text(label).size(theme::T_MD).font(fonts::UI).color(theme::FG_2),
            Space::new().width(Fill),
            container(text(keys).size(theme::T_SM).font(fonts::MONO)).padding([2, 8]).style(theme::keycap),
        ]
        .align_y(Alignment::Center)
        .into()
    }))
    .spacing(theme::S2);
    column![
        row![
            text("Keyboard shortcuts").size(18).font(fonts::UI_SEMIBOLD).color(theme::FG),
            Space::new().width(Fill),
            button(text("Done").size(theme::T_MD).font(fonts::UI_MEDIUM))
                .padding([6, 12])
                .style(theme::secondary_button)
                .on_press(Message::Modal(ModalMsg::Submit)),
        ]
        .align_y(Alignment::Center),
        rows,
    ]
    .spacing(theme::S4)
    .into()
}
