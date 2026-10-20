use super::{
    assets,
    theme::{self, Style},
};

/// Labels only; connector IDs and operations stay outside the presentation layer.
#[derive(Debug, Clone)]
pub struct Choice {
    pub label: String,
    pub hint: Option<String>,
}

pub struct Menu<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub choices: &'a [Choice],
}

/// A complete frame, without cursor movement. Narrow or short terminals get a compact logo.
/// Untrusted labels cannot inject terminal controls. Frames have no trailing newline so the
/// terminal adapter can redraw the same reserved rows without scrolling.
pub fn render(menu: &Menu<'_>, selected: usize, phase: usize, style: Style) -> String {
    let phase = if style.animate { phase } else { 0 };
    let mut rows = Vec::new();
    let wide = style.unicode && style.width >= 79 && style.height >= menu.choices.len() + 16;
    if wide {
        for (lettering, mark) in assets::WORDMARK.iter().zip(assets::MARK) {
            let lettering = paint(lettering, false, style);
            let mark = paint(mark, phase % 8 < 4, style);
            rows.push(format!("{lettering}    {mark}"));
        }
    } else {
        rows.push(format!(
            "{}    {}",
            paint("/goal-cup", false, style),
            paint("::", phase % 8 < 4, style)
        ));
    }
    rows.push(String::new());
    rows.push(paint(&clean(menu.title, style.width), true, style));
    rows.push(paint(&clean(menu.subtitle, style.width), false, style));
    rows.push(String::new());
    // Keep the selected item visible without wrapping the display on a short terminal.
    let room = style.height.saturating_sub(rows.len() + 2).max(1);
    let first = selected.saturating_sub(room.saturating_sub(1));
    for (index, choice) in menu.choices.iter().enumerate().skip(first).take(room) {
        let hint = choice
            .hint
            .as_deref()
            .map(|h| format!(" ({h})"))
            .unwrap_or_default();
        let label = clean(
            &format!(
                "{} {}{hint}",
                if index == selected { ">" } else { " " },
                choice.label
            ),
            style.width,
        );
        rows.push(paint(&label, index == selected, style));
    }
    rows.push(String::new());
    rows.push(paint(
        &clean("up/down select   enter continue   esc cancel", style.width),
        false,
        style,
    ));
    rows.join("\n")
}

fn paint(text: &str, bright: bool, style: Style) -> String {
    if !style.color {
        return text.to_owned();
    }
    format!(
        "{}{text}{}",
        if bright { theme::BRIGHT } else { theme::GREEN },
        theme::RESET
    )
}

fn clean(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .filter(|c| !c.is_control())
        .take_while(|c| {
            // Conservatively reserve two cells for non-ASCII labels, so wide glyphs cannot
            // wrap a frame. The renderer's own scanline banner has known single-cell glyphs.
            used += if c.is_ascii() { 1 } else { 2 };
            used <= width.saturating_sub(1)
        })
        .collect()
}
