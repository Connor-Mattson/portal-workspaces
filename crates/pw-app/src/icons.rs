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
    KEYBOARD => r#"<rect x="2" y="5" width="20" height="14" rx="2.5"/><path d="M6 9h.01M10 9h.01M14 9h.01M18 9h.01M6 13h.01M18 13h.01M9 16h6"/>"#,
    RESTART => r#"<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>"#,
    REFRESH => r#"<path d="M21 12a9 9 0 0 1-15.5 6.2L3 16"/><path d="M3 21v-5h5"/><path d="M3 12a9 9 0 0 1 15.5-6.2L21 8"/><path d="M21 3v5h-5"/>"#,
    GAUGE => r#"<path d="m12 14 4-4"/><path d="M3.34 19a10 10 0 1 1 17.32 0"/>"#,
}
