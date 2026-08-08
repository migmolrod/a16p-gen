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
