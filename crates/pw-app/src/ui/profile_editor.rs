//! The "Track an account" sheet: pick a provider, optionally say where the account lives (paste
//! your alias), and see right away which account that is.

use std::path::Path;

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Fill, Length, Task};
use pw_model::{ProfileId, Provider, UsageProfile};
use pw_usage::{Candidate, describe, discover};

use crate::app::Message;
use crate::fonts;
use crate::theme;
use crate::ui::modal::ModalMsg;

const NAME_INPUT: &str = "profile-name";

#[derive(Debug, Clone)]
pub enum ProfileMsg {
    Provider(Provider),
    Name(String),
    Instructions(String),
    UseCandidate(usize),
}

pub struct ProfileEditor {
    pub id: ProfileId,
    /// Editing an existing profile (vs. adding one).
    pub editing: bool,
    pub provider: Provider,
    pub name: String,
    pub instructions: String,
    /// Installs found on this machine that aren't tracked yet.
    pub candidates: Vec<Candidate>,
    /// What the current fields point at ("~/.claude-work · me@work.dev · Pro"), or what's wrong.
    pub preview: Result<String, String>,
    pub error: Option<String>,
    name_edited: bool,
    /// The other tracked profiles, to refuse a second profile for the same account.
    others: Vec<UsageProfile>,
}

impl ProfileEditor {
    pub fn new(tracked: &[UsageProfile]) -> Self {
        let home = crate::sessions::home_dir();
        let candidates: Vec<Candidate> =
            discover(home).into_iter().filter(|c| !tracked.iter().any(|t| same_source(t, &c.profile, home))).collect();
        let mut editor = Self {
            id: ProfileId::new(),
            editing: false,
            provider: Provider::Claude,
            name: String::new(),
            instructions: String::new(),
            candidates,
            preview: Ok(String::new()),
            error: None,
            name_edited: false,
            others: tracked.to_vec(),
        };
        match editor.candidates.first().map(|c| c.profile.clone()) {
            Some(first) => editor.fill_from(&first),
            None => editor.refresh(),
        }
        editor
    }

    pub fn edit(profile: &UsageProfile, tracked: &[UsageProfile]) -> Self {
        let mut editor = Self {
            id: profile.id,
            editing: true,
            provider: profile.provider,
            name: profile.name.clone(),
            instructions: profile.instructions.clone(),
            candidates: Vec::new(),
            preview: Ok(String::new()),
            error: None,
            name_edited: true,
            others: tracked.iter().filter(|p| p.id != profile.id).cloned().collect(),
        };
        editor.refresh();
        editor
    }

    pub fn update(&mut self, msg: ProfileMsg) {
        match msg {
            ProfileMsg::Provider(provider) => {
                if provider != self.provider {
                    self.provider = provider;
                    self.instructions.clear();
                    if !self.name_edited {
                        self.name = provider.label().to_owned();
                    }
                }
            }
            ProfileMsg::Name(name) => {
                self.name = name;
                self.name_edited = true;
            }
            ProfileMsg::Instructions(text) => self.instructions = text,
            ProfileMsg::UseCandidate(index) => {
                if let Some(profile) = self.candidates.get(index).map(|c| c.profile.clone()) {
                    self.fill_from(&profile);
                }
            }
        }
        self.error = None;
        self.refresh();
    }

    fn fill_from(&mut self, profile: &UsageProfile) {
        self.provider = profile.provider;
        self.name = profile.name.clone();
        self.instructions = profile.instructions.clone();
        self.name_edited = false;
        self.refresh();
    }

    fn draft(&self) -> UsageProfile {
        let name = match self.name.trim() {
            "" => self.provider.label().to_owned(),
            name => name.to_owned(),
        };
        UsageProfile { id: self.id, name, provider: self.provider, instructions: self.instructions.trim().to_owned() }
    }

    fn refresh(&mut self) {
        let home = crate::sessions::home_dir();
        let draft = self.draft();
        self.preview = match self.others.iter().find(|p| same_source(p, &draft, home)) {
            Some(twin) => Err(format!("That account is already tracked as “{}”.", twin.name)),
            None => describe(&draft, home),
        };
    }

    /// The profile to save, or why it can't be.
    pub fn validate(&self) -> Result<UsageProfile, String> {
        self.preview.clone()?;
        Ok(self.draft())
    }

    fn is_current(&self, candidate: &Candidate) -> bool {
        candidate.profile.provider == self.provider && candidate.profile.instructions == self.instructions.trim()
    }
}

/// Two profiles read the same account if they're the same provider in the same directory.
fn same_source(a: &UsageProfile, b: &UsageProfile, home: &Path) -> bool {
    a.provider == b.provider
        && match (a.env(home), b.env(home)) {
            (Ok(a), Ok(b)) => a.dir == b.dir,
            _ => false,
        }
}

pub fn focus() -> Task<Message> {
    iced::widget::operation::focus(NAME_INPUT)
}

fn msg(m: ProfileMsg) -> Message {
    Message::Modal(ModalMsg::Profile(m))
}

fn field_label(label: &str) -> Element<'_, Message> {
    text(label).size(theme::T_SM).font(fonts::UI_MEDIUM).color(theme::FG_2).into()
}

pub fn view(editor: &ProfileEditor) -> Element<'_, Message> {
    let title = if editor.editing { "Edit account" } else { "Track an account" };
    let subtitle = "Its 5-hour and weekly limits show in the drawer, checked every 2 minutes. \
                    Checking reads your usage; it never runs a model or uses tokens.";

    let mut fields = column![].spacing(theme::S4);

    if !editor.candidates.is_empty() {
        let found = column(editor.candidates.iter().enumerate().map(|(i, c)| {
            let active = editor.is_current(c);
            button(
                row![
                    text(c.profile.provider.label()).size(theme::T_XS).font(fonts::UI_SEMIBOLD).width(72),
                    column![
                        text(&c.profile.name).size(theme::T_SM).font(fonts::UI_MEDIUM),
                        text(&c.detail)
                            .size(theme::T_XS)
                            .font(fonts::UI)
                            .color(theme::FG_3)
                            .wrapping(text::Wrapping::None),
                    ]
                    .spacing(1),
                ]
                .spacing(theme::S2)
                .align_y(Alignment::Center),
            )
            .width(Fill)
            .padding([6, 10])
            .style(theme::chip(active))
            .on_press(msg(ProfileMsg::UseCandidate(i)))
            .into()
        }))
        .spacing(theme::S1);
        fields = fields.push(column![field_label("Found on this computer"), found].spacing(6));
    }

    let providers = row(Provider::ALL.into_iter().map(|p| {
        button(container(text(p.label()).size(theme::T_SM).font(fonts::UI_MEDIUM)).center_x(Fill))
            .width(Length::Fill)
            .padding([6, 0])
            .style(theme::chip(editor.provider == p))
            .on_press(msg(ProfileMsg::Provider(p)))
            .into()
    }))
    .spacing(theme::S1);
    fields = fields.push(column![field_label("Provider"), providers].spacing(6));

    let name = text_input(editor.provider.label(), &editor.name)
        .id(NAME_INPUT)
        .on_input(|s| msg(ProfileMsg::Name(s)))
        .on_submit(Message::Modal(ModalMsg::Submit))
        .padding([8, 10])
        .size(theme::T_MD)
        .font(fonts::UI)
        .style(theme::input);
    fields = fields.push(column![field_label("Name"), name].spacing(6));

    let (placeholder, default_dir) = match editor.provider {
        Provider::Claude => ("alias claude-work='CLAUDE_CONFIG_DIR=~/.claude-work claude'", "~/.claude"),
        Provider::Codex => ("CODEX_HOME=~/.codex-work", "~/.codex"),
        Provider::Antigravity => ("~/.gemini", "~/.gemini"),
    };
    let instructions = text_input(placeholder, &editor.instructions)
        .on_input(|s| msg(ProfileMsg::Instructions(s)))
        .on_submit(Message::Modal(ModalMsg::Submit))
        .padding([8, 10])
        .size(theme::T_SM)
        .font(fonts::MONO)
        .style(theme::input);
    let help = format!("Optional. Paste the alias or env you start it with, or a folder. Empty means {default_dir}.");
    let preview: Element<'_, Message> = match &editor.preview {
        Ok(found) if !found.is_empty() => {
            text(format!("→ {found}")).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::ACCENT).into()
        }
        Ok(_) => Space::new().into(),
        Err(problem) => text(problem).size(theme::T_XS).font(fonts::UI).color(theme::WARN).into(),
    };
    fields = fields.push(
        column![field_label("How to find it"), instructions, text(help).size(theme::T_XS).color(theme::FG_3), preview]
            .spacing(6),
    );

    if let Some(error) = &editor.error {
        fields = fields.push(
            container(text(error).size(theme::T_SM).font(fonts::UI))
                .padding([6, 10])
                .width(Fill)
                .style(theme::error_note),
        );
    }

    let mut actions = row![].spacing(theme::S2).align_y(Alignment::Center);
    if editor.editing {
        actions = actions.push(
            button(text("Stop tracking").size(theme::T_MD).font(fonts::UI_MEDIUM))
                .padding([7, 12])
                .style(theme::ghost_button)
                .on_press(Message::Modal(ModalMsg::Delete)),
        );
    }
    actions = actions.push(Space::new().width(Fill)).push(
        button(text("Cancel").size(theme::T_MD).font(fonts::UI_MEDIUM))
            .padding([7, 14])
            .style(theme::secondary_button)
            .on_press(Message::Modal(ModalMsg::Cancel)),
    );
    actions = actions.push(
        button(text(if editor.editing { "Save" } else { "Track account" }).size(theme::T_MD).font(fonts::UI_SEMIBOLD))
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
