//! The drawer's system profile: CPU, GPU, memory and network as a row of four rings.
//!
//! A ring fills with the share in use, in the accent, amber from 70% and red from 90% (like the
//! usage meters), with the number inside. Hovering a ring opens its card: a five-minute
//! sparkline, the details, which workspaces are using it (their terminals' process trees), and
//! the busiest programs. The collapsed drawer's rail shows the same rings as a 2×2 tile.
//!
//! Readings arrive every few seconds (see `pw_system::INTERVAL`), and only while the rings are on
//! screen; there's no timer here.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use iced::mouse::Cursor;
use iced::widget::canvas::{self, Geometry, LineCap, Path, Stroke};
use iced::widget::{Space, button, column, container, row, stack, svg, text, tooltip};
use iced::{Alignment, Color, Element, Fill, Length, Padding, Point, Radians, Rectangle, Renderer, Theme};
use pw_system::{INTERVAL, Process, Sample, Tree};

use crate::app::{App, Message};
use crate::fonts;
use crate::icons;
use crate::system::{HISTORY, Point as Reading};
use crate::theme;
use crate::ui::Drawn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Metric {
    Cpu,
    Gpu,
    Memory,
    Network,
}

impl Metric {
    const ALL: [Metric; 4] = [Metric::Cpu, Metric::Gpu, Metric::Memory, Metric::Network];

    fn label(self) -> &'static str {
        match self {
            Metric::Cpu => "CPU",
            Metric::Gpu => "GPU",
            Metric::Memory => "RAM",
            Metric::Network => "NET",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Metric::Cpu => "Processor",
            Metric::Gpu => "Graphics",
            Metric::Memory => "Memory",
            Metric::Network => "Network",
        }
    }
}

/// What a ring shows.
struct Gauge {
    /// Filled share, 0–1; `None` draws an empty track.
    fill: Option<f32>,
    color: Color,
    /// The number inside the ring.
    value: String,
}

fn gauge(metric: Metric, app: &App) -> Gauge {
    let Some(sample) = app.system.latest() else {
        return Gauge { fill: None, color: theme::FG_3, value: "…".into() };
    };
    let level = |used: f32| Gauge { fill: Some(used / 100.0), color: theme::level_color(used), value: percent(used) };
    match metric {
        Metric::Cpu => level(sample.cpu.usage),
        Metric::Memory => level(sample.memory.percent()),
        Metric::Gpu => match sample.gpu().and_then(|g| g.usage) {
            Some(used) => level(used),
            None => Gauge { fill: None, color: theme::FG_3, value: "—".into() },
        },
        Metric::Network => {
            let total = sample.network.rx + sample.network.tx;
            let value = rate_short(total);
            match sample.network.utilization() {
                Some(used) => Gauge { fill: Some(used / 100.0), color: theme::level_color(used), value },
                // No link speed (Wi-Fi, macOS): the ring is relative to the recent peak, which is
                // never a warning in itself.
                None => {
                    let peak = network_peak(app).max(1);
                    Gauge { fill: Some(total as f32 / peak as f32), color: theme::ACCENT, value }
                }
            }
        }
    }
}

fn network_peak(app: &App) -> u64 {
    app.system.history().iter().map(|p| p.network).max().unwrap_or(0)
}

// ---- drawer ------------------------------------------------------------------------------------

pub fn section(app: &App) -> Element<'_, Message> {
    let expanded = app.prefs.system_expanded;
    let toggle = button(
        row![
            svg(if expanded { icons::CHEVRON_DOWN.clone() } else { icons::CHEVRON_RIGHT.clone() })
                .width(12)
                .height(12)
                .style(theme::icon_hover(theme::FG_3, theme::FG)),
            text("SYSTEM").size(theme::T_XS).font(fonts::UI_SEMIBOLD),
        ]
        .spacing(theme::S1)
        .align_y(Alignment::Center),
    )
    .padding([3, 4])
    .style(theme::ghost_button)
    .on_press(Message::ToggleSystem);
    let header = container(toggle).padding(Padding::default().left(theme::S1));
    if !expanded {
        return header.into();
    }

    let owners = owners(app);
    let rings = row(Metric::ALL.map(|metric| {
        let g = gauge(metric, app);
        let cell = column![
            ring(&g, 40.0, 3.5, text(g.value.clone()).size(theme::T_XS).font(fonts::UI_SEMIBOLD).color(theme::FG)),
            text(metric.label()).size(10).font(fonts::UI_MEDIUM).color(theme::FG_3),
        ]
        .spacing(5)
        .align_x(Alignment::Center);
        // The whole cell (ring and label) is the hover target, not just the thin ring.
        let target = container(cell).width(Fill).center_x(Fill);
        tooltip(target, card(app, metric, &owners), tooltip::Position::Top).gap(8).into()
    }))
    .spacing(theme::S1);
    let body = container(rings).padding([10, 6]).width(Fill).style(theme::usage_card);
    column![header, body].spacing(theme::S1).into()
}

// ---- collapsed rail ----------------------------------------------------------------------------

/// The four rings as a 2×2 tile, each with its initial inside.
pub fn rail(app: &App) -> Element<'_, Message> {
    let owners = owners(app);
    let cell = |metric: Metric| -> Element<'_, Message> {
        let g = gauge(metric, app);
        // At this size the initial is drawn by the ring itself; a text layer over an 18 px canvas
        // didn't render.
        let program = Ring { fill: g.fill, color: g.color, thickness: 2.5, initial: Some(&metric.label()[..1]) };
        let mark = canvas::Canvas::new(program).width(18).height(18);
        tooltip(mark, card(app, metric, &owners), tooltip::Position::Right).gap(14).into()
    };
    let [cpu, gpu, memory, network] = Metric::ALL;
    container(
        column![row![cell(cpu), cell(gpu)].spacing(4), row![cell(memory), cell(network)].spacing(4)]
            .spacing(4)
            .align_x(Alignment::Center),
    )
    .padding(5)
    .style(theme::usage_card)
    .into()
}

// ---- hover cards -------------------------------------------------------------------------------

const CARD_WIDTH: f32 = 288.0;
/// Rows in a card's "by workspace" and "top programs" lists.
const LIST_ROWS: usize = 5;

/// Tooltip content is built on every view, shown or not, so a card only formats what it shows.
fn card<'a>(app: &'a App, metric: Metric, owners: &HashMap<u32, &str>) -> Element<'a, Message> {
    let Some(sample) = app.system.latest() else {
        return shell(column![header(metric.title(), "…".into(), None), note("Taking a first reading…")]);
    };
    let mut body = column![].spacing(10);
    match metric {
        Metric::Cpu => {
            let cpu = &sample.cpu;
            let mut sub = format!("{} threads", cpu.cores);
            if !cpu.brand.is_empty() {
                sub = format!("{} · {sub}", cpu.brand);
            }
            body = body.push(header(metric.title(), percent(cpu.usage), Some(sub)));
            body = body.push(sparkline(app, |p| Some(p.cpu), 100.0, percent));
            if let Some([one, five, fifteen]) = cpu.load {
                body = body.push(facts(vec![("Load average".into(), format!("{one:.2} · {five:.2} · {fifteen:.2}"))]));
            }
            body = body.push(workspaces(sample, owners, |t| t.cpu as f64, |v| percent(v as f32)));
            body = body.push(programs(sample, owners, |p| p.cpu as f64, |v| percent(v as f32), true));
        }
        Metric::Memory => {
            let m = &sample.memory;
            let sub = format!("{} of {} in use", bytes(m.used), bytes(m.total));
            body = body.push(header(metric.title(), percent(m.percent()), Some(sub)));
            body = body.push(sparkline(app, |p| Some(p.memory), 100.0, percent));
            if m.swap_total > 0 {
                body = body
                    .push(facts(vec![("Swap".into(), format!("{} of {}", bytes(m.swap_used), bytes(m.swap_total)))]));
            }
            body = body.push(workspaces(sample, owners, |t| t.memory as f64, |v| bytes(v as u64)));
            body = body.push(programs(sample, owners, |p| p.memory as f64, |v| bytes(v as u64), true));
        }
        Metric::Gpu => {
            let Some(top) = sample.gpu() else {
                let why = sample.gpu_note.clone().unwrap_or_default();
                return shell(column![header(metric.title(), "—".into(), None), note(why)].spacing(6));
            };
            let value = top.usage.map_or_else(|| "—".into(), percent);
            let sub = (sample.gpus.len() == 1).then(|| top.name.clone());
            body = body.push(header(metric.title(), value, sub));
            body = body.push(sparkline(app, |p| p.gpu, 100.0, percent));
            for gpu in &sample.gpus {
                let mut lines = Vec::new();
                if sample.gpus.len() > 1 {
                    lines.push((gpu.name.clone(), gpu.usage.map_or_else(|| "—".into(), percent)));
                }
                if let Some(used) = gpu.memory_used {
                    let memory = match gpu.memory_total {
                        Some(total) => format!("{} of {}", bytes(used), bytes(total)),
                        None => bytes(used),
                    };
                    lines.push(("Memory".into(), memory));
                }
                let sensors: Vec<String> =
                    [gpu.temperature.map(|t| format!("{t}°C")), gpu.power_watts.map(|w| format!("{w:.0} W"))]
                        .into_iter()
                        .flatten()
                        .collect();
                if !sensors.is_empty() {
                    lines.push(("Temperature · power".into(), sensors.join(" · ")));
                }
                body = body.push(facts(lines));
            }
            body = body.push(workspaces(sample, owners, |t| t.gpu_memory as f64, |v| bytes(v as u64)));
            body =
                body.push(programs(sample, owners, |p| p.gpu_memory.unwrap_or(0) as f64, |v| bytes(v as u64), false));
        }
        Metric::Network => {
            let net = &sample.network;
            let sub = format!("↓ {}  ·  ↑ {}", rate(net.rx), rate(net.tx));
            body = body.push(header(metric.title(), rate(net.rx + net.tx), Some(sub)));
            let peak = network_peak(app).max(1) as f32;
            body = body.push(sparkline(app, |p| Some(p.network as f32), peak, |v| rate(v as u64)));
            if net.interfaces.is_empty() {
                body = body.push(note("No network hardware found."));
            }
            let lines = net
                .interfaces
                .iter()
                .map(|i| {
                    let mut name = i.name.clone();
                    if let Some(link) = i.link {
                        name = format!("{name} · {}", link_speed(link));
                    }
                    (name, format!("↓ {}  ↑ {}", rate_short(i.rx), rate_short(i.tx)))
                })
                .collect();
            body = body.push(facts(lines));
        }
    }
    let age = sample.at.elapsed().unwrap_or_default();
    body = body.push(note(format!("Every {} s · updated {}", INTERVAL.as_secs(), ago(age))));
    shell(body)
}

fn shell<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content).padding([10, 12]).width(CARD_WIDTH).style(theme::hover_card).into()
}

fn header<'a>(title: &'a str, value: String, subtitle: Option<String>) -> Element<'a, Message> {
    let top = row![
        text(title).size(theme::T_SM).font(fonts::UI_SEMIBOLD).color(theme::FG).width(Fill),
        text(value).size(theme::T_SM).font(fonts::UI_SEMIBOLD).color(theme::FG),
    ]
    .align_y(Alignment::Center);
    let mut col = column![top].spacing(2);
    if let Some(sub) = subtitle {
        col = col.push(text(sub).size(theme::T_XS).font(fonts::UI).color(theme::FG_3));
    }
    col.into()
}

fn note<'a>(s: impl text::IntoFragment<'a>) -> Element<'a, Message> {
    text(s).size(theme::T_XS).font(fonts::UI).color(theme::FG_3).into()
}

/// Label on the left, value on the right.
fn facts<'a>(lines: Vec<(String, String)>) -> Element<'a, Message> {
    column(lines.into_iter().map(|(label, value)| {
        row![
            text(label).size(theme::T_XS).font(fonts::UI).color(theme::FG_3).width(Fill),
            text(value).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::FG_2),
        ]
        .spacing(theme::S2)
        .into()
    }))
    .spacing(3)
    .into()
}

/// A titled list; nothing at all when it has no rows.
fn list<'a>(title: &'a str, rows: Vec<(String, Option<String>, String)>) -> Element<'a, Message> {
    if rows.is_empty() {
        return Space::new().into();
    }
    let rows = rows.into_iter().map(|(name, tag, value)| {
        let mut label =
            row![text(name).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::FG_2).wrapping(text::Wrapping::None)]
                .spacing(5);
        if let Some(tag) = tag {
            label = label
                .push(text(tag).size(theme::T_XS).font(fonts::UI).color(theme::FG_3).wrapping(text::Wrapping::None));
        }
        row![
            container(label).width(Fill).clip(true),
            text(value).size(theme::T_XS).font(fonts::UI_MEDIUM).color(theme::FG_2),
        ]
        .spacing(theme::S2)
        .into()
    });
    column![text(title).size(10).font(fonts::UI_SEMIBOLD).color(theme::FG_3), column(rows).spacing(3),]
        .spacing(4)
        .into()
}

/// Which workspace each tracked shell belongs to, by pid.
fn owners(app: &App) -> HashMap<u32, &str> {
    let mut owners = HashMap::new();
    for ws in &app.workspaces {
        for pane in ws.all_pane_ids().into_iter().chain([ws.editor_terminal()]) {
            if let Some(session) = app.sessions.get(pane).and_then(|rt| rt.session.as_ref()) {
                owners.insert(session.pid(), ws.model.name.as_str());
            }
        }
    }
    owners
}

/// Each workspace's terminals totalled, busiest first. Idle workspaces are left out.
fn workspaces<'a>(
    sample: &Sample,
    owners: &HashMap<u32, &str>,
    key: impl Fn(&Tree) -> f64,
    show: impl Fn(f64) -> String,
) -> Element<'a, Message> {
    let mut totals: Vec<(&str, f64)> = Vec::new();
    for tree in &sample.trees {
        let Some(&name) = owners.get(&tree.root) else { continue };
        match totals.iter_mut().find(|(n, _)| *n == name) {
            Some((_, total)) => *total += key(tree),
            None => totals.push((name, key(tree))),
        }
    }
    totals.retain(|(_, v)| *v > 0.0);
    totals.sort_by(|a, b| b.1.total_cmp(&a.1));
    let rows = totals.into_iter().take(LIST_ROWS).map(|(name, v)| (name.to_owned(), None, show(v))).collect();
    list("BY WORKSPACE", rows)
}

/// The busiest programs by `key`, each tagged with its workspace when it runs in one.
fn programs<'a>(
    sample: &Sample,
    owners: &HashMap<u32, &str>,
    key: impl Fn(&Process) -> f64,
    show: impl Fn(f64) -> String,
    counted: bool,
) -> Element<'a, Message> {
    let mut top: Vec<&Process> = sample.processes.iter().filter(|p| key(p) > 0.0).collect();
    top.sort_by(|a, b| key(b).total_cmp(&key(a)));
    let rows = top
        .into_iter()
        .take(LIST_ROWS)
        .map(|p| {
            // "×N" counts the whole group; for GPU memory usually only one of them holds any.
            let name = if counted && p.count > 1 { format!("{} ×{}", p.name, p.count) } else { p.name.clone() };
            let tag = p.root.and_then(|r| owners.get(&r)).map(|ws| format!("in {ws}"));
            (name, tag, show(key(p)))
        })
        .collect();
    list("TOP PROGRAMS", rows)
}

// ---- drawing -----------------------------------------------------------------------------------

/// A ring of `size` px with `center` inside it.
fn ring<'a>(gauge: &Gauge, size: f32, thickness: f32, center: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    let program = Ring { fill: gauge.fill, color: gauge.color, thickness, initial: None };
    stack![canvas::Canvas::new(program).width(size).height(size), container(center).center(Length::Fixed(size))].into()
}

#[derive(Clone, PartialEq)]
struct Ring {
    fill: Option<f32>,
    color: Color,
    thickness: f32,
    /// A letter in the middle, for rings too small for a number.
    initial: Option<&'static str>,
}

impl canvas::Program<Message> for Ring {
    type State = Drawn<Ring>;

    fn draw(&self, drawn: &Self::State, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: Cursor) -> Vec<Geometry> {
        vec![drawn.draw(renderer, bounds.size(), self, |frame| {
            let center = frame.center();
            let radius = bounds.width.min(bounds.height) / 2.0 - self.thickness / 2.0;
            let stroke =
                |color| Stroke::default().with_width(self.thickness).with_color(color).with_line_cap(LineCap::Round);
            frame.stroke(&Path::circle(center, radius), stroke(theme::RING_TRACK));
            // Anything above zero shows at least a dot, so "a little" never reads as "nothing".
            if let Some(fill) = self.fill.filter(|f| *f > 0.0).map(|f| f.clamp(0.01, 1.0)) {
                let start = -std::f32::consts::FRAC_PI_2;
                let arc = Path::new(|b| {
                    b.arc(canvas::path::Arc {
                        center,
                        radius,
                        start_angle: Radians(start),
                        end_angle: Radians(start + fill * std::f32::consts::TAU),
                    });
                });
                frame.stroke(&arc, stroke(self.color));
            }
            if let Some(initial) = self.initial {
                frame.fill_text(canvas::Text {
                    content: initial.to_owned(),
                    position: center,
                    color: theme::FG_3,
                    size: 9.0.into(),
                    font: fonts::UI_SEMIBOLD,
                    align_x: iced::widget::text::Alignment::Center,
                    align_y: iced::alignment::Vertical::Center,
                    ..canvas::Text::default()
                });
            }
        })]
    }
}

/// The last few minutes of one measure, scaled to `max`, with its peak.
fn sparkline<'a>(
    app: &App,
    value: impl Fn(&Reading) -> Option<f32>,
    max: f32,
    show: impl Fn(f32) -> String,
) -> Element<'a, Message> {
    // The newest reading is the right edge, not the clock, so the chart only changes when a reading arrives.
    let now = app.system.history().back().map_or_else(SystemTime::now, |p| p.at);
    let span = HISTORY.as_secs_f32();
    let mut segments: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut last: Option<SystemTime> = None;
    let mut peak: Option<f32> = None;
    for point in app.system.history() {
        let Some(v) = value(point) else {
            last = None;
            continue;
        };
        peak = Some(peak.map_or(v, |p: f32| p.max(v)));
        let age = now.duration_since(point.at).unwrap_or_default().as_secs_f32();
        let x = (1.0 - age / span).clamp(0.0, 1.0);
        let y = (v / max.max(f32::EPSILON)).clamp(0.0, 1.0);
        // A gap (sampling was paused) breaks the line rather than bridging it.
        let joined = last.is_some_and(|t| point.at.duration_since(t).unwrap_or_default() < INTERVAL * 3);
        match segments.last_mut() {
            Some(segment) if joined => segment.push((x, y)),
            _ => segments.push(vec![(x, y)]),
        }
        last = Some(point.at);
    }
    let chart = canvas::Canvas::new(Spark { segments, color: theme::ACCENT }).width(Fill).height(34);
    let minutes = HISTORY.as_secs() / 60;
    let caption = row![
        text(format!("Last {minutes} min")).size(10).font(fonts::UI).color(theme::FG_3).width(Fill),
        text(peak.map(|p| format!("Peak {}", show(p))).unwrap_or_default()).size(10).font(fonts::UI).color(theme::FG_3),
    ];
    column![chart, caption].spacing(3).into()
}

#[derive(Clone, PartialEq)]
struct Spark {
    /// Runs of (x, y) in 0–1, x from five minutes before the newest reading to it, y up from the baseline.
    segments: Vec<Vec<(f32, f32)>>,
    color: Color,
}

impl canvas::Program<Message> for Spark {
    type State = Drawn<Spark>;

    fn draw(&self, drawn: &Self::State, renderer: &Renderer, _: &Theme, bounds: Rectangle, _: Cursor) -> Vec<Geometry> {
        vec![drawn.draw(renderer, bounds.size(), self, |frame| {
            let (w, h) = (bounds.width, bounds.height);
            let line = 1.5;
            let at = |(x, y): (f32, f32)| Point::new(x * w, h - line / 2.0 - y * (h - line));
            let baseline = Path::line(Point::new(0.0, h - 0.5), Point::new(w, h - 0.5));
            frame.stroke(&baseline, Stroke::default().with_width(1.0).with_color(theme::LINE));
            for segment in &self.segments {
                match segment.as_slice() {
                    [] => {}
                    [only] => frame.fill(&Path::circle(at(*only), 1.5), self.color),
                    [first, .., last] => {
                        let area = Path::new(|b| {
                            b.move_to(Point::new(first.0 * w, h));
                            for p in segment {
                                b.line_to(at(*p));
                            }
                            b.line_to(Point::new(last.0 * w, h));
                            b.close();
                        });
                        frame.fill(&area, theme::alpha(self.color, 0.14));
                        let trace = Path::new(|b| {
                            b.move_to(at(*first));
                            for p in &segment[1..] {
                                b.line_to(at(*p));
                            }
                        });
                        let stroke =
                            Stroke::default().with_width(line).with_color(self.color).with_line_cap(LineCap::Round);
                        frame.stroke(&trace, stroke);
                    }
                }
            }
        })]
    }
}

// ---- numbers -----------------------------------------------------------------------------------

/// "37%", and "0.4%" for small shares so a quiet process doesn't read as zero.
fn percent(v: f32) -> String {
    if v > 0.0 && v < 9.95 { format!("{v:.1}%") } else { format!("{v:.0}%") }
}

/// "512 B", "84.2 KB", "10.2 GB", "980 MB" (binary units, as system monitors show them).
fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    match unit {
        0 => format!("{n} B"),
        _ if value < 100.0 => format!("{value:.1} {}", UNITS[unit]),
        _ => format!("{value:.0} {}", UNITS[unit]),
    }
}

fn rate(n: u64) -> String {
    format!("{}/s", bytes(n))
}

/// A rate short enough for the inside of a ring: "0", "512B", "84K", "1.2M".
fn rate_short(n: u64) -> String {
    let full = bytes(n);
    let (number, unit) = full.split_once(' ').unwrap_or((&full, ""));
    match unit {
        "B" if n == 0 => "0".into(),
        "B" => format!("{number}B"),
        _ => {
            // One decimal only while it's a single digit: "1.2M", "84K".
            let number = match number.split_once('.') {
                Some((whole, _)) if whole.len() > 1 => whole,
                _ => number,
            };
            format!("{number}{}", &unit[..1])
        }
    }
}

/// "1 Gbit/s", "2.5 Gbit/s", "100 Mbit/s".
fn link_speed(bytes_per_sec: u64) -> String {
    let mbit = bytes_per_sec * 8 / 1_000_000;
    if mbit >= 1000 {
        let gbit = mbit as f64 / 1000.0;
        if gbit.fract() == 0.0 { format!("{gbit:.0} Gbit/s") } else { format!("{gbit:.1} Gbit/s") }
    } else {
        format!("{mbit} Mbit/s")
    }
}

fn ago(d: Duration) -> String {
    match d.as_secs() {
        0..2 => "just now".into(),
        s @ 2..60 => format!("{s} s ago"),
        s => format!("{} min ago", s / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(percent(0.0), "0%");
        assert_eq!(percent(0.42), "0.4%");
        assert_eq!(percent(37.4), "37%");
        assert_eq!(percent(100.0), "100%");
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(84_207), "82.2 KB");
        assert_eq!(bytes(13_472_985_088), "12.5 GB");
        assert_eq!(bytes(67_224_825_856), "62.6 GB");
        assert_eq!(bytes(700 * 1024 * 1024), "700 MB");
        assert_eq!(rate_short(0), "0");
        assert_eq!(rate_short(999), "999B");
        assert_eq!(rate_short(84_207), "82K");
        assert_eq!(rate_short(1_300_000), "1.2M");
        assert_eq!(link_speed(125_000_000), "1 Gbit/s");
        assert_eq!(link_speed(312_500_000), "2.5 Gbit/s");
        assert_eq!(link_speed(12_500_000), "100 Mbit/s");
        assert_eq!(ago(Duration::from_secs(4)), "4 s ago");
    }
}
