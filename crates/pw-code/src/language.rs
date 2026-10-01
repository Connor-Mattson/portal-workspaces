//! What the editor knows about each kind of file: how to recognise it, how comments and pairs work in it,
//! which syntax highlights it, and how it is indented by default.
//!
//! Languages are rows of one table ([`TABLE`]), so supporting another one is one entry.

use std::path::Path;

/// A colour family for a file's icon. The app maps each to a palette colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tint {
    Orange,
    Blue,
    Yellow,
    Green,
    Purple,
    Red,
    Teal,
    Gray,
}

/// How a file is indented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Indent {
    Tabs,
    Spaces(u8),
}

impl Indent {
    /// One level of indentation as text.
    pub fn unit(self) -> String {
        match self {
            Indent::Tabs => "\t".to_owned(),
            Indent::Spaces(n) => " ".repeat(n as usize),
        }
    }

    pub fn label(self) -> String {
        match self {
            Indent::Tabs => "Tabs".to_owned(),
            Indent::Spaces(n) => format!("Spaces: {n}"),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Spec {
    /// Shown in the status bar.
    pub name: &'static str,
    /// File extensions, lowercase, without the dot.
    pub extensions: &'static [&'static str],
    /// Exact file names (`Makefile`, `Dockerfile`).
    pub file_names: &'static [&'static str],
    /// Interpreters named on a `#!` line.
    pub interpreters: &'static [&'static str],
    /// The extension that selects its syntax definition. Empty for none (plain text).
    pub syntax: &'static str,
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    /// Quote characters that pair up and delimit strings.
    pub quotes: &'static str,
    pub indent: Indent,
    pub tint: Tint,
}

/// A language: a row of [`TABLE`]. Cheap to copy and compare.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Language(&'static Spec);

impl std::fmt::Debug for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.name)
    }
}

impl std::hash::Hash for Language {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.name.hash(state);
    }
}

impl std::ops::Deref for Language {
    type Target = Spec;

    fn deref(&self) -> &Spec {
        self.0
    }
}

impl Language {
    pub const PLAIN: Language = Language(&PLAIN);

    /// The language of a file, from its name, then its extension, then a `#!` first line.
    pub fn detect(path: &Path, first_line: Option<&str>) -> Language {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
        if let Some(spec) = TABLE.iter().find(|s| s.file_names.contains(&name)) {
            return Language(spec);
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        if !ext.is_empty()
            && let Some(spec) = TABLE.iter().find(|s| s.extensions.contains(&ext.as_str()))
        {
            return Language(spec);
        }
        first_line.and_then(Self::from_shebang).unwrap_or(Self::PLAIN)
    }

    fn from_shebang(line: &str) -> Option<Language> {
        let command = line.strip_prefix("#!")?.trim();
        let mut words = command.split_whitespace();
        let program = words.next()?.rsplit('/').next()?;
        let program = if program == "env" { words.find(|w| !w.starts_with('-'))? } else { program };
        let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        TABLE.iter().find(|s| s.interpreters.contains(&program)).map(Language)
    }

    pub fn is_plain(self) -> bool {
        self == Self::PLAIN
    }

    /// The closing character for a bracket or quote typed in this language.
    pub fn closer(self, open: char) -> Option<char> {
        match open {
            '{' => Some('}'),
            '[' => Some(']'),
            '(' => Some(')'),
            q if self.quotes.contains(q) => Some(q),
            _ => None,
        }
    }
}

const PLAIN: Spec = Spec {
    name: "Plain Text",
    extensions: &["txt"],
    file_names: &[],
    interpreters: &[],
    syntax: "",
    line_comment: None,
    block_comment: None,
    quotes: "\"",
    indent: Indent::Spaces(4),
    tint: Tint::Gray,
};

const C_LIKE: Option<(&str, &str)> = Some(("/*", "*/"));

/// Every language the editor knows, plain text last.
pub static TABLE: &[Spec] = &[
    Spec {
        name: "Rust",
        extensions: &["rs"],
        file_names: &[],
        interpreters: &[],
        syntax: "rs",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"",
        indent: Indent::Spaces(4),
        tint: Tint::Orange,
    },
    Spec {
        name: "Python",
        extensions: &["py", "pyi", "pyw"],
        file_names: &[],
        interpreters: &["python", "python3"],
        syntax: "py",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Blue,
    },
    Spec {
        name: "TypeScript",
        extensions: &["ts", "mts", "cts"],
        file_names: &[],
        interpreters: &["deno", "ts-node"],
        syntax: "ts",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'`",
        indent: Indent::Spaces(2),
        tint: Tint::Blue,
    },
    Spec {
        name: "TSX",
        extensions: &["tsx"],
        file_names: &[],
        interpreters: &[],
        syntax: "tsx",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'`",
        indent: Indent::Spaces(2),
        tint: Tint::Blue,
    },
    Spec {
        name: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
        file_names: &[],
        interpreters: &["node", "bun"],
        syntax: "js",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'`",
        indent: Indent::Spaces(2),
        tint: Tint::Yellow,
    },
    Spec {
        name: "JSON",
        extensions: &["json", "jsonc", "json5", "jsonl", "ipynb"],
        file_names: &[".prettierrc", ".eslintrc"],
        interpreters: &[],
        syntax: "json",
        line_comment: Some("//"),
        block_comment: None,
        quotes: "\"",
        indent: Indent::Spaces(2),
        tint: Tint::Yellow,
    },
    Spec {
        name: "TOML",
        extensions: &["toml"],
        file_names: &["Cargo.lock", "uv.lock", "poetry.lock"],
        interpreters: &[],
        syntax: "toml",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Gray,
    },
    Spec {
        name: "YAML",
        extensions: &["yaml", "yml"],
        file_names: &[],
        interpreters: &[],
        syntax: "yaml",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Purple,
    },
    Spec {
        name: "Markdown",
        extensions: &["md", "markdown", "mdx"],
        file_names: &[],
        interpreters: &[],
        syntax: "md",
        line_comment: None,
        block_comment: Some(("<!--", "-->")),
        quotes: "`",
        indent: Indent::Spaces(2),
        tint: Tint::Teal,
    },
    Spec {
        name: "HTML",
        extensions: &["html", "htm", "xhtml", "vue", "svelte"],
        file_names: &[],
        interpreters: &[],
        syntax: "html",
        line_comment: None,
        block_comment: Some(("<!--", "-->")),
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Orange,
    },
    Spec {
        name: "CSS",
        extensions: &["css"],
        file_names: &[],
        interpreters: &[],
        syntax: "css",
        line_comment: None,
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Purple,
    },
    Spec {
        name: "SCSS",
        extensions: &["scss", "sass", "less"],
        file_names: &[],
        interpreters: &[],
        syntax: "scss",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Purple,
    },
    Spec {
        name: "Shell",
        extensions: &["sh", "bash", "zsh", "fish", "ksh"],
        file_names: &[".bashrc", ".zshrc", ".profile", ".bash_profile", ".envrc", "PKGBUILD"],
        interpreters: &["sh", "bash", "zsh", "dash", "ksh", "fish"],
        syntax: "sh",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'`",
        indent: Indent::Spaces(2),
        tint: Tint::Green,
    },
    Spec {
        name: "C",
        extensions: &["c", "h"],
        file_names: &[],
        interpreters: &[],
        syntax: "c",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Blue,
    },
    Spec {
        name: "C++",
        extensions: &["cpp", "cc", "cxx", "hpp", "hh", "hxx", "cu", "cuh", "ino"],
        file_names: &[],
        interpreters: &[],
        syntax: "cpp",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Blue,
    },
    Spec {
        name: "C#",
        extensions: &["cs", "csx"],
        file_names: &[],
        interpreters: &[],
        syntax: "cs",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Purple,
    },
    Spec {
        name: "Go",
        extensions: &["go"],
        file_names: &["go.mod", "go.sum", "go.work"],
        interpreters: &[],
        syntax: "go",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'`",
        indent: Indent::Tabs,
        tint: Tint::Teal,
    },
    Spec {
        name: "Java",
        extensions: &["java"],
        file_names: &[],
        interpreters: &[],
        syntax: "java",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Red,
    },
    Spec {
        name: "Kotlin",
        extensions: &["kt", "kts"],
        file_names: &[],
        interpreters: &[],
        syntax: "kt",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Purple,
    },
    Spec {
        name: "Swift",
        extensions: &["swift"],
        file_names: &[],
        interpreters: &[],
        syntax: "swift",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"",
        indent: Indent::Spaces(4),
        tint: Tint::Orange,
    },
    Spec {
        name: "Ruby",
        extensions: &["rb", "rake", "gemspec"],
        file_names: &["Gemfile", "Rakefile"],
        interpreters: &["ruby"],
        syntax: "rb",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Red,
    },
    Spec {
        name: "PHP",
        extensions: &["php"],
        file_names: &[],
        interpreters: &["php"],
        syntax: "php",
        line_comment: Some("//"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Purple,
    },
    Spec {
        name: "Lua",
        extensions: &["lua"],
        file_names: &[],
        interpreters: &["lua", "luajit"],
        syntax: "lua",
        line_comment: Some("--"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Blue,
    },
    Spec {
        name: "Zig",
        extensions: &["zig", "zon"],
        file_names: &[],
        interpreters: &[],
        syntax: "zig",
        line_comment: Some("//"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Orange,
    },
    Spec {
        name: "Haskell",
        extensions: &["hs", "lhs"],
        file_names: &[],
        interpreters: &["runhaskell"],
        syntax: "hs",
        line_comment: Some("--"),
        block_comment: Some(("{-", "-}")),
        quotes: "\"",
        indent: Indent::Spaces(2),
        tint: Tint::Purple,
    },
    Spec {
        name: "Elixir",
        extensions: &["ex", "exs"],
        file_names: &[],
        interpreters: &["elixir"],
        syntax: "ex",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Purple,
    },
    Spec {
        name: "SQL",
        extensions: &["sql"],
        file_names: &[],
        interpreters: &[],
        syntax: "sql",
        line_comment: Some("--"),
        block_comment: C_LIKE,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Yellow,
    },
    Spec {
        name: "XML",
        extensions: &["xml", "svg", "plist", "xsd", "xsl", "csproj"],
        file_names: &[],
        interpreters: &[],
        syntax: "xml",
        line_comment: None,
        block_comment: Some(("<!--", "-->")),
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Orange,
    },
    Spec {
        name: "Dockerfile",
        extensions: &["dockerfile", "containerfile"],
        file_names: &["Dockerfile", "Containerfile"],
        interpreters: &[],
        syntax: "dockerfile",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(4),
        tint: Tint::Blue,
    },
    Spec {
        name: "Makefile",
        extensions: &["mk", "mak"],
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        interpreters: &["make"],
        syntax: "mk",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Tabs,
        tint: Tint::Gray,
    },
    Spec {
        name: "CMake",
        extensions: &["cmake"],
        file_names: &["CMakeLists.txt"],
        interpreters: &[],
        syntax: "cmake",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"",
        indent: Indent::Spaces(4),
        tint: Tint::Gray,
    },
    Spec {
        name: "Nix",
        extensions: &["nix"],
        file_names: &[],
        interpreters: &[],
        syntax: "nix",
        line_comment: Some("#"),
        block_comment: C_LIKE,
        quotes: "\"",
        indent: Indent::Spaces(2),
        tint: Tint::Blue,
    },
    Spec {
        name: "LaTeX",
        extensions: &["tex", "sty", "cls", "bib"],
        file_names: &[],
        interpreters: &[],
        syntax: "tex",
        line_comment: Some("%"),
        block_comment: None,
        quotes: "",
        indent: Indent::Spaces(2),
        tint: Tint::Teal,
    },
    Spec {
        name: "Diff",
        extensions: &["diff", "patch"],
        file_names: &[],
        interpreters: &[],
        syntax: "diff",
        line_comment: None,
        block_comment: None,
        quotes: "",
        indent: Indent::Spaces(4),
        tint: Tint::Green,
    },
    Spec {
        name: "INI",
        extensions: &["ini", "cfg", "conf", "properties", "editorconfig"],
        file_names: &[".editorconfig", ".gitconfig", ".npmrc"],
        interpreters: &[],
        syntax: "ini",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"",
        indent: Indent::Spaces(4),
        tint: Tint::Gray,
    },
    Spec {
        name: "Ignore",
        extensions: &[],
        file_names: &[".gitignore", ".dockerignore", ".ignore", ".prettierignore", ".gitattributes"],
        interpreters: &[],
        syntax: "gitignore",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "",
        indent: Indent::Spaces(4),
        tint: Tint::Gray,
    },
    Spec {
        name: "Env",
        extensions: &["env"],
        file_names: &[".env", ".env.local", ".env.example", ".env.development", ".env.production"],
        interpreters: &[],
        syntax: "sh",
        line_comment: Some("#"),
        block_comment: None,
        quotes: "\"'",
        indent: Indent::Spaces(2),
        tint: Tint::Yellow,
    },
    PLAIN,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn name(path: &str, first: Option<&str>) -> &'static str {
        Language::detect(Path::new(path), first).name
    }

    #[test]
    fn detects_by_name_extension_and_shebang() {
        assert_eq!(name("src/main.rs", None), "Rust");
        assert_eq!(name("App.TSX", None), "TSX");
        assert_eq!(name("Dockerfile", None), "Dockerfile");
        assert_eq!(name("crates/Cargo.lock", None), "TOML");
        assert_eq!(name("bin/run", Some("#!/usr/bin/env python3")), "Python");
        assert_eq!(name("bin/run", Some("#!/bin/bash -e")), "Shell");
        assert_eq!(name("bin/run", Some("#!/usr/bin/env -S node --flag")), "JavaScript");
        assert_eq!(name("notes", None), "Plain Text");
        assert!(Language::detect(Path::new("x.unknown"), None).is_plain());
    }

    #[test]
    fn closers_follow_the_language_quotes() {
        let rust = Language::detect(Path::new("a.rs"), None);
        assert_eq!(rust.closer('('), Some(')'));
        assert_eq!(rust.closer('"'), Some('"'));
        // Lifetimes and chars: `'` doesn't pair in Rust.
        assert_eq!(rust.closer('\''), None);
        let py = Language::detect(Path::new("a.py"), None);
        assert_eq!(py.closer('\''), Some('\''));
    }

    #[test]
    fn table_is_unambiguous() {
        let mut seen = std::collections::HashSet::new();
        for spec in TABLE {
            for ext in spec.extensions {
                assert!(seen.insert(*ext), "extension {ext} listed twice");
                assert_eq!(*ext, ext.to_ascii_lowercase());
            }
        }
        assert_eq!(TABLE.last(), Some(&PLAIN));
    }

    #[test]
    fn indent_units() {
        assert_eq!(Indent::Spaces(2).unit(), "  ");
        assert_eq!(Indent::Tabs.unit(), "\t");
    }
}
