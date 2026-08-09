use crate::palette_gen::Swatch;
use std::collections::BTreeMap;

fn bg_block(s: &Swatch) -> String {
    format!("\x1b[48;2;{};{};{}m  \x1b[0m", s.rgb[0], s.rgb[1], s.rgb[2])
}

pub fn print_ansi16(resolved: &BTreeMap<String, Swatch>) {
    println!("background/foreground:");
    if let (Some(bg), Some(fg)) = (resolved.get("background"), resolved.get("foreground")) {
        print!("{}", bg_block(bg));
        print!("{}", bg_block(fg));
        println!();
    }

    println!("ansi 0-7:");
    for i in 0..8 {
        if let Some(s) = resolved.get(&format!("ansi_color{i}")) {
            print!("{}", bg_block(s));
        }
    }
    println!();

    println!("ansi 8-15 (bright):");
    for i in 8..16 {
        if let Some(s) = resolved.get(&format!("ansi_color{i}")) {
            print!("{}", bg_block(s));
        }
    }
    println!();
}

const MOCK_WIDTH: usize = 56;

fn fg_escape(s: &Swatch) -> String {
    format!("\x1b[38;2;{};{};{}m", s.rgb[0], s.rgb[1], s.rgb[2])
}

/// Render one terminal-width line: `bg` fills the whole row (including the
/// padding), each `(fg, text)` span is colored with `fg` (falling back to
/// the ambient foreground when `None`, e.g. for a bright/dim variant that
/// isn't present in a slimmed-down semantic mapping).
fn mock_line(bg: &Swatch, fg: &Swatch, spans: &[(Option<&Swatch>, &str)]) -> String {
    let visible_len: usize = spans.iter().map(|(_, t)| t.chars().count()).sum();
    let pad = MOCK_WIDTH.saturating_sub(visible_len);
    let mut out = format!("\x1b[48;2;{};{};{}m", bg.rgb[0], bg.rgb[1], bg.rgb[2]);
    for (span_fg, text) in spans {
        out.push_str(&fg_escape(span_fg.unwrap_or(fg)));
        out.push_str(text);
    }
    out.push_str(&" ".repeat(pad));
    out.push_str("\x1b[0m");
    out
}

/// A rough terminal mockup (prompt, `ls`, log levels, a diff, a code line)
/// so bg/fg/ansi colors can be judged together the way they'd actually be
/// seen, instead of as isolated swatch blocks -- isolated blocks make it
/// hard to tell if e.g. foreground-on-background contrast actually works.
pub fn print_terminal_mock(resolved: &BTreeMap<String, Swatch>) {
    let (Some(bg), Some(fg)) = (resolved.get("background"), resolved.get("foreground")) else {
        return;
    };
    let get = |name: &str| resolved.get(name);
    let red = get("ansi_color1");
    let green = get("ansi_color2");
    let yellow = get("ansi_color3");
    let blue = get("ansi_color4");
    let magenta = get("ansi_color5");
    let cyan = get("ansi_color6");

    println!("terminal mockup:");
    let lines: &[&[(Option<&Swatch>, &str)]] = &[
        &[
            (green, "user@host"),
            (None, ":"),
            (blue, "~/dotfiles"),
            (None, "$ ls -la"),
        ],
        &[(blue, "drwxr-xr-x"), (None, "  config/")],
        &[(green, "-rwxr-xr-x"), (None, "  build.sh")],
        &[(None, "-rw-r--r--  README.md")],
        &[(blue, "[INFO]"), (None, " service started")],
        &[(yellow, "[WARN]"), (None, " cache miss")],
        &[(red, "[ERROR]"), (None, " connection refused")],
        &[(green, "+ added line")],
        &[(red, "- removed line")],
        &[(magenta, "fn"), (None, " main"), (cyan, "()"), (None, " {")],
        &[
            (green, "user@host"),
            (None, ":"),
            (blue, "~/dotfiles"),
            (None, "$ "),
        ],
    ];
    for spans in lines {
        println!("{}", mock_line(bg, fg, spans));
    }
}
