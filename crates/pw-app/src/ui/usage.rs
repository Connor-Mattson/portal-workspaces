//! The drawer's usage section: one card per tracked AI account, with a meter per limit window.
//!
//! A meter is a thin bar (accent, amber from 70%, red from 90%) with a tick marking how much of
//! the window's time has passed: fill to the right of the tick means you're ahead of pace. The
//! collapsed rail shows the same two headline numbers as a pair of vertical bars.
//!
//! Time text ("2h 10m", "Thu 9am") is computed at draw time, so it stays current on every redraw
//! (readings arrive every 2 minutes, and the cwd check redraws every 10 seconds) without a timer
//! of its own.

use std::time::{Duration, SystemTime};

use iced::widget::{Space, button, column, container, hover, row, scrollable, stack, svg, text, tooltip};
use iced::{Alignment, Color, Element, Fill, Length, Padding};
use pw_model::UsageProfile;
use pw_usage::{Failure, Span, Window};

use crate::app::{App, Message};
use crate::fonts;
use crate::icons;
use crate::theme;
use crate::ui::icon_button;
use crate::usage::ProfileStatus;

/// Meter resolution: fill and pace are drawn in thousandths.
const STEPS: u16 = 1000;

pub fn section(app: &App) -> Element<'_, Message> {
    let expanded = app.prefs.usage_expanded;
    let profiles = app.usage.profiles();
    let toggle = button(
        row![
            svg(if expanded { icons::CHEVRON_DOWN.clone() } else { icons::CHEVRON_RIGHT.clone() })
                .width(12)
                .height(12)
                .style(theme::icon_hover(theme::FG_3, theme::FG)),
            text("USAGE").size(theme::T_XS).font(fonts::UI_SEMIBOLD),
        ]
        .spacing(theme::S1)
        .align_y(Alignment::Center),
    )
    .padding([3, 4])
    .style(theme::ghost_button)
    .on_press(Message::ToggleUsage);

    let header = row![
        toggle,
        Space::new().width(Fill),
        icon_button(&icons::REFRESH, 13.0, "Check now", (!profiles.is_empty()).then_some(Message::RefreshUsage)),
        icon_button(&icons::PLUS, 15.0, "Track an account", Some(Message::AddProfile)),
    ]
    .align_y(Alignment::Center);

    if !expanded {
        return container(header).padding(Padding::default().left(theme::S1)).into();
    }
    let body: Element<'_, Message> = if profiles.is_empty() {
        empty_state()
    } else {
        let cards = column(profiles.iter().map(|p| card(app, p))).spacing(theme::S1 + 2.0);
        container(scrollable(cards).spacing(theme::S1).style(theme::scroller)).max_height(520).into()
    };
    column![container(header).padding(Padding::default().left(theme::S1)), body].spacing(theme::S1).into()
}

fn empty_state<'a>() -> Element<'a, Message> {
    let add = button(
        row![
            svg(icons::GAUGE.clone()).width(14).height(14).style(theme::icon(theme::ACCENT)),
            text("Track an account").size(theme::T_SM).font(fonts::UI_MEDIUM),
        ]
        .spacing(theme::S2)
        .align_y(Alignment::Center),
    )
    .padding([5, 10])
    .style(theme::secondary_button)
    .on_press(Message::AddProfile);
    container(
        column![
            text("See your Claude, Codex and Antigravity 5-hour and weekly limits here.")
                .size(theme::T_XS)
                .font(fonts::UI)
                .color(theme::FG_3),
            add,
        ]
        .spacing(theme::S2),
    )
    .padding([10, 10])
    .width(Fill)
    .style(theme::usage_card)
    .into()
}

fn card<'a>(app: &'a App, profile: &'a UsageProfile) -> Element<'a, Message> {
    let now = SystemTime::now();
    let status = app.usage.status(profile.id);
    let report = status.and_then(|s| s.last.as_ref());
    let stale = status.is_some_and(|s| s.failure.is_some());
    let (meta, meta_color) = meta(status, now);

    let title = row![
        container(
            text(&profile.name)
                .size(theme::T_SM)
                .font(fonts::UI_SEMIBOLD)
                .color(theme::FG)
                .wrapping(text::Wrapping::None)
        )
        .width(Fill)
        .clip(true),
        text(meta).size(theme::T_XS).font(fonts::UI).color(meta_color).wrapping(text::Wrapping::None),
    ]
    .spacing(theme::S2)
    .align_y(Alignment::Center);

    let mut body = column![title].spacing(5);
    match (report, status.and_then(|s| s.failure.as_ref())) {
        (Some(report), _) => {
            let mut scope: Option<&str> = None;
            for window in report.windows.iter().filter(|w| shown(w, now)) {
                if window.scope.as_deref() != scope {
                    scope = window.scope.as_deref();
                    body = body.push(caption(scope.unwrap_or("All models")));
                }
                body = body.push(meter_row(window, now, stale));
            }
        }
        // An expired sign-in gets the renew row below instead.
        (None, Some(Failure::Idle { .. })) => {}
        (None, Some(failure)) => {
            body = body.push(text(failure.to_string()).size(theme::T_XS).font(fonts::UI).color(failure_color(failure)));
        }
        (None, None) => {
            body = body.push(skeleton_row(Span::FiveHour)).push(skeleton_row(Span::Weekly));
        }
    }
    if let Some(status) = status
        && let Some(Failure::Idle { hint }) = &status.failure
    {
        body = body.push(renew_row(profile, status, hint));
    }

    let card = container(body).padding([8, 10]).width(Fill).style(theme::usage_card);
    // Hover reveals the edit control over the card's top-right corner, like the workspace rows.
    let edit = button(
        container(svg(icons::PENCIL.clone()).width(12).height(12).style(theme::icon_hover(theme::FG_3, theme::FG)))
            .center(Length::Fixed(22.0)),
    )
    .padding(0)
    .style(theme::ghost_button)
    .on_press(Message::EditProfile(profile.id));
    let controls = container(container(edit).style(theme::keycap)).align_right(Fill).padding([5, 6]);

    tooltip(hover(card, controls), details(profile, status, now), tooltip::Position::Right).gap(10).into()
}

/// "Sign-in expired" with a button that has the CLI renew it in the background, so you don't have
/// to go run it yourself. Only if that doesn't work does it ask you to.
fn renew_row<'a>(profile: &UsageProfile, status: &ProfileStatus, hint: &str) -> Element<'a, Message> {
    let note = match (status.renewing, status.renew_failed) {
        (true, _) => "Renewing sign-in…".to_owned(),
        (false, true) => format!("Still expired. Run {hint} once to sign in."),
        (false, false) => "Sign-in expired.".to_owned(),
    };
    let renew = button(
        row![
            svg(icons::REFRESH.clone()).width(11).height(11).style(theme::icon_hover(theme::FG_3, theme::FG)),
            text(if status.renew_failed { "Retry" } else { "Renew" }).size(theme::T_XS).font(fonts::UI_MEDIUM),
        ]
        .spacing(4)
        .align_y(Alignment::Center),
    )
    .padding([2, 6])
    .style(theme::ghost_button)
    .on_press_maybe((!status.renewing).then_some(Message::RenewProfile(profile.id)));
    row![text(note).size(theme::T_XS).font(fonts::UI).color(theme::WARN).width(Fill), renew]
        .spacing(theme::S2)
        .align_y(Alignment::Center)
        .into()
}

/// Scoped windows that haven't been touched (no usage, no window open) are left out, so a
/// model family you don't use doesn't take up the card. Account-wide windows always show.
fn shown(window: &Window, now: SystemTime) -> bool {
    window.scope.is_none() || window.used_at(now) > 0.0 || window.resets_in(now).is_some()
}

/// A scope heading inside a card ("Gemini", "Opus"), shown when a provider splits its limits.
fn caption(scope: &str) -> Element<'_, Message> {
    text(scope).size(10).font(fonts::UI_MEDIUM).color(theme::FG_3).into()
}

fn meter_row<'a>(window: &Window, now: SystemTime, stale: bool) -> Element<'a, Message> {
    let used = window.used_at(now);
    row![
        text(window.span.label()).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::FG_3).width(30),
        meter(used, window.elapsed_at(now), stale),
        text(format!("{used:.0}%"))
            .size(theme::T_XS)
            .font(fonts::UI_MEDIUM)
            .color(if stale { theme::FG_3 } else { theme::FG_2 })
            .width(30)
            .align_x(iced::alignment::Horizontal::Right),
        text(reset_label(window, now))
            .size(theme::T_XS)
            .font(fonts::UI)
            .color(theme::FG_3)
            .width(50)
            .align_x(iced::alignment::Horizontal::Right)
            .wrapping(text::Wrapping::None),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn skeleton_row<'a>(span: Span) -> Element<'a, Message> {
    row![
        text(span.label()).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::FG_3).width(30),
        meter(0.0, None, true),
        text("…").size(theme::T_XS).color(theme::FG_3).width(86),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

/// A 4 px bar with a pace tick, 8 px tall overall.
fn meter<'a>(used: f32, elapsed: Option<f32>, stale: bool) -> Element<'a, Message> {
    let color = theme::faded(theme::level_color(used), stale);
    let fill = container(Space::new().width(Fill).height(Fill)).style(theme::meter_fill(color));
    let track = container(split(steps(used / 100.0), fill.into(), Space::new().width(Fill).into(), true))
        .width(Fill)
        .height(4)
        .style(theme::meter_track);
    let mut layers = stack![container(track).width(Fill).height(8).center_y(8)];
    if let Some(elapsed) = elapsed.filter(|_| !stale) {
        let tick = container(Space::new().width(2).height(8)).style(theme::pace_tick);
        let offset = steps(elapsed);
        layers = layers.push(row![
            Space::new().width(Length::FillPortion(offset.max(1))),
            tick,
            Space::new().width(Length::FillPortion((STEPS - offset).max(1))),
        ]);
    }
    layers.width(Fill).height(8).into()
}

fn steps(fraction: f32) -> u16 {
    (fraction.clamp(0.0, 1.0) * f32::from(STEPS)).round() as u16
}

/// `first` takes `n` thousandths of the space (`horizontal`) or height, `second` the rest.
fn split<'a>(
    n: u16,
    first: Element<'a, Message>,
    second: Element<'a, Message>,
    horizontal: bool,
) -> Element<'a, Message> {
    let part = |e: Element<'a, Message>, portion: u16| {
        let c = container(e);
        if horizontal {
            c.width(Length::FillPortion(portion)).height(Fill)
        } else {
            c.height(Length::FillPortion(portion)).width(Fill)
        }
    };
    match n {
        0 => part(second, 1).into(),
        n if n >= STEPS => part(first, 1).into(),
        n if horizontal => row![part(first, n), part(second, STEPS - n)].into(),
        n => column![part(first, n), part(second, STEPS - n)].into(),
    }
}

/// The card's top-right note: plan and freshness, or what's wrong.
fn meta(status: Option<&ProfileStatus>, now: SystemTime) -> (String, Color) {
    let Some(status) = status else { return ("checking…".into(), theme::FG_3) };
    match (&status.failure, &status.last) {
        (Some(failure), _) => (
            match failure {
                Failure::Idle { .. } => "idle",
                Failure::SignedOut(_) => "signed out",
                Failure::NotFound(_) => "not found",
                Failure::Unavailable(_) => "retrying",
                Failure::RateLimited { .. } => "paused",
            }
            .into(),
            failure_color(failure),
        ),
        (None, Some(report)) => {
            let age = ago(now.duration_since(report.as_of).unwrap_or_default());
            let fresh = if report.live { age } else { format!("as of {age}") };
            let text = match &report.plan {
                Some(plan) => format!("{plan} · {fresh}"),
                None => fresh,
            };
            (text, theme::FG_3)
        }
        (None, None) => ("checking…".into(), theme::FG_3),
    }
}

fn failure_color(failure: &Failure) -> Color {
    match failure {
        Failure::SignedOut(_) | Failure::NotFound(_) => theme::BAD,
        _ => theme::WARN,
    }
}

/// The full story, on hover: every window with its exact reset time, the account, freshness.
fn details<'a>(profile: &'a UsageProfile, status: Option<&'a ProfileStatus>, now: SystemTime) -> Element<'a, Message> {
    let line = |s: String, color: Color| text(s).size(theme::T_XS).font(fonts::UI).color(color);
    let report = status.and_then(|s| s.last.as_ref());
    let mut heading = profile.provider.label().to_owned();
    if let Some(account) = report.and_then(|r| r.account.as_deref()) {
        heading = format!("{heading} · {account}");
    }
    let mut lines = column![
        text(&profile.name).size(theme::T_SM).font(fonts::UI_SEMIBOLD).color(theme::FG),
        line(heading, theme::FG_3),
    ]
    .spacing(3);
    if let Some(report) = report {
        for w in &report.windows {
            let reset = match (w.resets_at, w.resets_in(now)) {
                (Some(at), Some(left)) => format!(" · resets {} ({})", clock(at, left), duration(left)),
                (Some(_), None) => " · reset since".to_owned(),
                (None, _) => " · not started".to_owned(),
            };
            lines = lines.push(line(format!("{}: {:.0}% used{reset}", w.title(), w.used_at(now)), theme::FG_2));
        }
        let age = ago(now.duration_since(report.as_of).unwrap_or_default());
        let source =
            if report.live { format!("Updated {age} ago") } else { format!("Last recorded by the CLI {age} ago") };
        lines = lines.push(line(source, theme::FG_3));
    }
    if let Some(failure) = status.and_then(|s| s.failure.as_ref()) {
        lines = lines.push(line(failure.to_string(), failure_color(failure)));
    }
    if let Some(next) = status.and_then(|s| s.next_poll).and_then(|t| t.duration_since(now).ok()) {
        lines = lines.push(line(format!("Next check in {}", duration(next)), theme::FG_3));
    }
    container(lines).padding([8, 10]).max_width(320).style(theme::keycap).into()
}

// ---- collapsed rail ------------------------------------------------------------------------

/// Each profile as two small vertical bars (5-hour, weekly) with its initials.
pub fn rail(app: &App) -> Element<'_, Message> {
    let now = SystemTime::now();
    column(app.usage.profiles().iter().map(|profile| {
        let status = app.usage.status(profile.id);
        let report = status.and_then(|s| s.last.as_ref());
        let stale = status.is_some_and(|s| s.failure.is_some()) || report.is_none();
        let bar = |span| vbar(report.and_then(|r| r.window(span)).map_or(0.0, |w| w.used_at(now)), stale);
        let initials: String = profile.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect();
        let tile = container(
            column![
                row![bar(Span::FiveHour), bar(Span::Weekly)].spacing(3),
                text(initials.to_uppercase()).size(9).font(fonts::UI_SEMIBOLD).color(theme::FG_3),
            ]
            .spacing(3)
            .align_x(Alignment::Center),
        )
        .padding([5, 0])
        .width(36)
        .align_x(Alignment::Center)
        .style(theme::usage_card);
        tooltip(tile, details(profile, status, now), tooltip::Position::Right).gap(8).into()
    }))
    .spacing(theme::S1 + 2.0)
    .align_x(Alignment::Center)
    .into()
}

fn vbar<'a>(used: f32, stale: bool) -> Element<'a, Message> {
    let color = theme::faded(theme::level_color(used), stale);
    let fill = container(Space::new().width(Fill).height(Fill)).style(theme::meter_fill(color));
    let n = steps(used / 100.0);
    container(split(STEPS - n, Space::new().width(Fill).into(), fill.into(), false))
        .width(5)
        .height(22)
        .style(theme::meter_track)
        .into()
}

// ---- time text ---------------------------------------------------------------------------------

/// When a window resets, as short as it can be: "42m", "2h 10m", "4d 2h". The tooltip has the
/// exact time.
fn reset_label(window: &Window, now: SystemTime) -> String {
    match (window.resets_at, window.resets_in(now)) {
        (_, Some(left)) => duration(left),
        (Some(_), None) => "reset".into(),
        (None, None) => String::new(),
    }
}

fn duration(d: Duration) -> String {
    let minutes = d.as_secs().div_ceil(60);
    match minutes {
        0..60 => format!("{minutes}m"),
        60..1440 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, (minutes % 1440) / 60),
    }
}

fn ago(d: Duration) -> String {
    match d.as_secs() {
        0..60 => "now".into(),
        s => duration(Duration::from_secs(s - s % 60)),
    }
}

fn local(at: SystemTime) -> Option<jiff::Zoned> {
    Some(jiff::Timestamp::try_from(at).ok()?.to_zoned(jiff::tz::TimeZone::system()))
}

/// "9am", "4:30pm".
fn time_of_day(z: &jiff::Zoned) -> String {
    if z.minute() == 0 { z.strftime("%-I%P").to_string() } else { z.strftime("%-I:%M%P").to_string() }
}

/// "Thu 9am" in local time.
fn weekday(at: SystemTime) -> String {
    local(at).map_or_else(String::new, |z| format!("{} {}", z.strftime("%a"), time_of_day(&z)))
}

/// An exact local time: "4:10pm" today, else "Thu 9am".
fn clock(at: SystemTime, left: Duration) -> String {
    match local(at) {
        Some(z) if left < Duration::from_secs(20 * 3600) => time_of_day(&z),
        Some(_) => weekday(at),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_are_short() {
        assert_eq!(duration(Duration::from_secs(30)), "1m");
        assert_eq!(duration(Duration::from_secs(42 * 60)), "42m");
        assert_eq!(duration(Duration::from_secs(2 * 3600 + 10 * 60)), "2h 10m");
        assert_eq!(duration(Duration::from_secs(3 * 86400 + 4 * 3600)), "3d 4h");
        assert_eq!(ago(Duration::from_secs(20)), "now");
        assert_eq!(ago(Duration::from_secs(150)), "2m");
    }

    #[test]
    fn local_times_read_naturally() {
        let z: jiff::Zoned = "2026-10-01T09:00:00[UTC]".parse().unwrap();
        assert_eq!(time_of_day(&z), "9am");
        let z: jiff::Zoned = "2026-10-01T16:30:00[UTC]".parse().unwrap();
        assert_eq!(time_of_day(&z), "4:30pm");
        assert_eq!(z.strftime("%a").to_string(), "Thu");
    }

    #[test]
    fn reset_labels() {
        let now = SystemTime::now();
        let soon = Window::new(Span::FiveHour, None, 10.0, Some(now + Duration::from_secs(90 * 60 + 5)));
        assert_eq!(reset_label(&soon, now), "1h 31m");
        let gone = Window::new(Span::FiveHour, None, 10.0, Some(now - Duration::from_secs(1)));
        assert_eq!(reset_label(&gone, now), "reset");
        let far = Window::new(Span::Weekly, None, 10.0, Some(now + Duration::from_secs(3 * 86400)));
        assert_eq!(reset_label(&far, now), "3d 0h");
        assert_eq!(reset_label(&Window::new(Span::Weekly, None, 0.0, None), now), "");
    }

    #[test]
    fn steps_clamp() {
        assert_eq!(steps(-1.0), 0);
        assert_eq!(steps(0.4215), 422);
        assert_eq!(steps(2.0), STEPS);
    }
}
