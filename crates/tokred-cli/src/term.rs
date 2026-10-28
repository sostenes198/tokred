//! Terminal styling and layout: ANSI colors, bars and aligned tables.
//!
//! Everything here returns plain `String`s so screens can be unit-tested; whether color is
//! used is decided once (see `Io::color`) and carried in [`Term`].

const RESET: &str = "\x1b[0m";

/// Style switch for one output stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Term {
    color: bool,
}

impl Term {
    /// A terminal with (`true`) or without ANSI colors.
    #[must_use]
    pub const fn new(color: bool) -> Self {
        Self { color }
    }

    fn paint(self, code: &str, text: &str) -> String {
        if self.color && !text.is_empty() {
            format!("\x1b[{code}m{text}{RESET}")
        } else {
            text.to_owned()
        }
    }

    /// Bold text.
    #[must_use]
    pub fn bold(self, text: &str) -> String {
        self.paint("1", text)
    }

    /// Bold cyan: titles and names.
    #[must_use]
    pub fn accent(self, text: &str) -> String {
        self.paint("1;36", text)
    }

    /// Cyan.
    #[must_use]
    pub fn cyan(self, text: &str) -> String {
        self.paint("36", text)
    }

    /// Dim text: secondary information.
    #[must_use]
    pub fn dim(self, text: &str) -> String {
        self.paint("2", text)
    }

    /// Yellow: something to look at.
    #[must_use]
    pub fn yellow(self, text: &str) -> String {
        self.paint("33", text)
    }

    /// Red: a problem.
    #[must_use]
    pub fn red(self, text: &str) -> String {
        self.paint("31", text)
    }

    /// Colors `text` by how good `fraction` (0..=1) is: green from 60%, yellow from 25%, else red.
    #[must_use]
    pub fn heat(self, fraction: f64, text: &str) -> String {
        let code = if fraction >= 0.6 {
            "32"
        } else if fraction >= 0.25 {
            "33"
        } else {
            "31"
        };
        self.paint(code, text)
    }

    /// A bar of `width` cells with `fraction` (clamped to 0..=1) filled, in the accent color.
    #[must_use]
    pub fn bar(self, fraction: f64, width: usize) -> String {
        let (filled, rest) = bar_cells(fraction, width);
        format!(
            "{}{}",
            self.cyan(&"█".repeat(filled)),
            self.dim(&"░".repeat(rest))
        )
    }

    /// Like [`Term::bar`] but colored by [`Term::heat`].
    #[must_use]
    pub fn meter(self, fraction: f64, width: usize) -> String {
        let (filled, rest) = bar_cells(fraction, width);
        format!(
            "{}{}",
            self.heat(fraction, &"█".repeat(filled)),
            self.dim(&"░".repeat(rest))
        )
    }
}

/// How many cells of a `width`-cell bar are filled and empty.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn bar_cells(fraction: f64, width: usize) -> (usize, usize) {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((fraction * width as f64).round() as usize).min(width);
    (filled, width - filled)
}

/// Width of `text` on screen: ANSI escape sequences take no space.
#[must_use]
pub fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip a CSI sequence: ESC [ ... final byte in @..~
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) && next != '[' {
                    break;
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

/// Shortens `text` to at most `max` characters, ending with `...` when it was cut.
#[must_use]
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let keep = max.saturating_sub(3);
    let mut cut: String = text.chars().take(keep).collect();
    cut.push_str("...");
    cut
}

/// A title with a double rule under it, as in `rtk gain`.
#[must_use]
pub fn title(term: Term, text: &str) -> String {
    let rule = "═".repeat(visible_width(text).max(24));
    format!("{}\n{}\n", term.accent(text), term.bold(&rule))
}

/// Aligned `label: value` lines; values start in one column.
#[must_use]
pub fn key_values(rows: &[(&str, String)]) -> String {
    let width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0)
        + 1;
    rows.iter()
        .map(|(label, value)| format!("{:<width$} {value}\n", format!("{label}:")))
        .collect()
}

/// Horizontal alignment of a table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// Text columns.
    Left,
    /// Numeric columns.
    Right,
}

/// A table column.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    /// Header text.
    pub title: &'static str,
    /// Alignment of header and cells.
    pub align: Align,
}

fn pad(text: &str, width: usize, align: Align) -> String {
    let gap = width.saturating_sub(visible_width(text));
    match align {
        Align::Left => format!("{text}{}", " ".repeat(gap)),
        Align::Right => format!("{}{text}", " ".repeat(gap)),
    }
}

/// A table with a rule above the header, under it and at the bottom. Cells may contain ANSI
/// sequences; widths are computed on what is visible.
#[must_use]
pub fn table(term: Term, columns: &[Column], rows: &[Vec<String>]) -> String {
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            rows.iter()
                .filter_map(|row| row.get(index))
                .map(|cell| visible_width(cell))
                .chain(std::iter::once(column.title.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: Vec<String>| cells.join("  ");
    let total = widths.iter().sum::<usize>() + 2 * widths.len().saturating_sub(1);
    let rule = term.dim(&"─".repeat(total));

    let header = line(
        columns
            .iter()
            .zip(&widths)
            .map(|(column, width)| term.bold(&pad(column.title, *width, column.align)))
            .collect(),
    );
    let mut out = format!("{rule}\n{header}\n{rule}\n");
    for row in rows {
        let cells = columns
            .iter()
            .zip(&widths)
            .enumerate()
            .map(|(index, (column, width))| {
                pad(
                    row.get(index).map_or("", String::as_str),
                    *width,
                    column.align,
                )
            })
            .collect();
        out.push_str(&line(cells));
        out.push('\n');
    }
    out.push_str(&rule);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ON: Term = Term::new(true);
    const OFF: Term = Term::new(false);

    #[test]
    fn colors_are_only_emitted_when_enabled() {
        assert_eq!(OFF.bold("x"), "x");
        assert_eq!(ON.bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(ON.accent("x"), "\x1b[1;36mx\x1b[0m");
        assert_eq!(ON.cyan("x"), "\x1b[36mx\x1b[0m");
        assert_eq!(ON.dim("x"), "\x1b[2mx\x1b[0m");
        assert_eq!(ON.yellow("x"), "\x1b[33mx\x1b[0m");
        assert_eq!(ON.red("x"), "\x1b[31mx\x1b[0m");
        assert_eq!(ON.dim(""), "", "nothing to paint");
    }

    #[test]
    fn heat_goes_from_red_to_yellow_to_green() {
        assert_eq!(ON.heat(0.1, "x"), "\x1b[31mx\x1b[0m");
        assert_eq!(ON.heat(0.25, "x"), "\x1b[33mx\x1b[0m");
        assert_eq!(ON.heat(0.6, "x"), "\x1b[32mx\x1b[0m");
        assert_eq!(OFF.heat(0.9, "x"), "x");
    }

    #[test]
    fn bars_fill_proportionally_and_clamp() {
        assert_eq!(OFF.bar(0.5, 10), "█████░░░░░");
        assert_eq!(OFF.bar(0.0, 4), "░░░░");
        assert_eq!(OFF.bar(2.0, 4), "████");
        assert_eq!(OFF.bar(-1.0, 4), "░░░░");
        assert_eq!(OFF.bar(f64::NAN, 4), "░░░░");
        assert_eq!(OFF.meter(0.3, 10), "███░░░░░░░");
        assert!(ON.meter(0.9, 10).starts_with("\x1b[32m"));
        assert!(ON.bar(0.5, 10).starts_with("\x1b[36m"));
    }

    #[test]
    fn visible_width_ignores_ansi() {
        assert_eq!(visible_width("abc"), 3);
        assert_eq!(visible_width(&ON.accent("abc")), 3);
        assert_eq!(visible_width(&ON.bar(0.5, 8)), 8);
        assert_eq!(visible_width("───"), 3);
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("exactly-10", 10), "exactly-10");
        assert_eq!(truncate("a-very-long-name", 10), "a-very-...");
        assert_eq!(truncate("abcdef", 2), "...");
    }

    #[test]
    fn titles_have_a_double_rule_at_least_as_wide_as_the_text() {
        let plain = title(OFF, "tokred Token Savings (last 7 days)");
        let mut lines = plain.lines();
        assert_eq!(lines.next(), Some("tokred Token Savings (last 7 days)"));
        assert_eq!(lines.next().map(|l| l.chars().count()), Some(34));
        let short = title(OFF, "Hi");
        assert_eq!(short.lines().nth(1).map(|l| l.chars().count()), Some(24));
        assert!(title(ON, "Hi").contains("\x1b[1;36mHi"));
    }

    #[test]
    fn key_values_align_their_values() {
        let text = key_values(&[("Real cost", "$1".to_owned()), ("Tokens", "5".to_owned())]);
        assert_eq!(text, "Real cost: $1\nTokens:    5\n");
        assert_eq!(key_values(&[]), "");
    }

    #[test]
    fn tables_align_columns_and_survive_colored_cells() {
        let columns = [
            Column {
                title: "#",
                align: Align::Right,
            },
            Column {
                title: "Name",
                align: Align::Left,
            },
            Column {
                title: "Cost",
                align: Align::Right,
            },
        ];
        let rows = vec![
            vec!["1.".to_owned(), "opus".to_owned(), "$10.00".to_owned()],
            vec!["10.".to_owned(), "haiku".to_owned()],
        ];
        let text = table(OFF, &columns, &rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[1], "  #  Name     Cost");
        assert_eq!(lines[3], " 1.  opus   $10.00");
        assert_eq!(lines[4], "10.  haiku        ");
        assert!(lines[0].chars().all(|c| c == '─') && lines[0] == lines[2] && lines[2] == lines[5]);

        // Colored cells keep the same layout.
        let colored = vec![vec!["1.".to_owned(), ON.accent("opus"), "$1".to_owned()]];
        let text = table(ON, &columns, &colored);
        let row = text.lines().nth(3).unwrap();
        assert_eq!(
            visible_width(row),
            visible_width(text.lines().nth(1).unwrap())
        );
    }

    #[test]
    fn an_empty_table_still_has_its_frame() {
        let columns = [Column {
            title: "A",
            align: Align::Left,
        }];
        assert_eq!(table(OFF, &columns, &[]).lines().count(), 4);
    }
}
