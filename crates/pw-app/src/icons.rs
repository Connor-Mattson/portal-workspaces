//! Small stroked UI icons (24×24 grid, Lucide-style), tinted at draw time.

use std::sync::LazyLock;

use iced::widget::svg::Handle;

macro_rules! icons {
    ($($name:ident => $body:literal),* $(,)?) => {
        $(
            pub static $name: LazyLock<Handle> = LazyLock::new(|| {
                Handle::from_memory(concat!(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round">"#,
                    $body,
                    "</svg>"
                ).as_bytes())
            });
        )*
    };
}

icons! {
    PLUS => r#"<path d="M12 5v14M5 12h14"/>"#,
    CLOSE => r#"<path d="M18 6 6 18M6 6l12 12"/>"#,
    SPLIT_RIGHT => r#"<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M12 4v16"/>"#,
    SPLIT_DOWN => r#"<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 12h18"/>"#,
    MAXIMIZE => r#"<path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7"/>"#,
    MINIMIZE => r#"<path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7"/>"#,
    POP_OUT => r#"<path d="M21 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h6"/><path d="M15 3h6v6M21 3l-9 9"/>"#,
    DOCK => r#"<path d="M13 3h6a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-6"/><path d="M3 3l9 9M12 6v6H6"/>"#,
    SIDEBAR => r#"<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M9 4v16"/>"#,
    CHEVRON_UP => r#"<path d="m18 15-6-6-6 6"/>"#,
    CHEVRON_DOWN => r#"<path d="m6 9 6 6 6-6"/>"#,
    CHEVRON_RIGHT => r#"<path d="m9 18 6-6-6-6"/>"#,
    PENCIL => r#"<path d="M17 3a2.85 2.85 0 0 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/>"#,
    FOLDER => r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/>"#,
    TERMINAL => r#"<path d="m5 8 5 4-5 4M12 17h7"/>"#,
    BELL => r#"<path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/>"#,
    BELL_OFF => r#"<path d="M8.7 3A6 6 0 0 1 18 8a21.3 21.3 0 0 0 .6 5"/><path d="M17 17H3s3-2 3-9a4.67 4.67 0 0 1 .3-1.7"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/><path d="m2 2 20 20"/>"#,
    KEYBOARD => r#"<rect x="2" y="5" width="20" height="14" rx="2.5"/><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M18 13h.01M9 16h6"/>"#,
    RESTART => r#"<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>"#,
    REFRESH => r#"<path d="M21 12a9 9 0 0 1-15.5 6.2L3 16"/><path d="M3 21v-5h5"/><path d="M3 12a9 9 0 0 1 15.5-6.2L21 8"/><path d="M21 3v5h-5"/>"#,
    GAUGE => r#"<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>"#,
    // Editor mode.
    CODE => r#"<path d="m16 18 6-6-6-6M8 6l-6 6 6 6"/>"#,
    LAYOUT_GRID => r#"<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/>"#,
    FILE => r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v5h6"/>"#,
    FILES => r#"<path d="M20 7h-3a2 2 0 0 1-2-2V2"/><path d="M9 18a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h7l4 4v10a2 2 0 0 1-2 2Z"/><path d="M3 7.6v12.8A1.6 1.6 0 0 0 4.6 22h9.8"/>"#,
    FOLDER_OPEN => r#"<path d="m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2"/>"#,
    FILE_PLUS => r#"<path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v5h6M12 12v6M9 15h6"/>"#,
    FOLDER_PLUS => r#"<path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/><path d="M12 10v6M9 13h6"/>"#,
    COLLAPSE => r#"<path d="m7 20 5-5 5 5M7 4l5 5 5-5"/>"#,
    SEARCH => r#"<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>"#,
    ARROW_UP => r#"<path d="M12 19V5M5 12l7-7 7 7"/>"#,
    ARROW_DOWN => r#"<path d="M12 5v14M5 12l7 7 7-7"/>"#,
    TRASH => r#"<path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>"#,
    PANEL_BOTTOM => r#"<rect x="3" y="4" width="18" height="16" rx="2.5"/><path d="M3 14h18"/>"#,
    WARNING => r#"<path d="m21.7 18-8-14a2 2 0 0 0-3.4 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.7-3"/><path d="M12 9v4M12 17h.01"/>"#,
}
