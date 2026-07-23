//! Coloring for the CLI's plain status lines — the `Compiled …`, `[watch] …`,
//! `hmr: …`, `error:`/`warning:` prefixes, and the test-runner summary. The
//! ariadne diagnostics color themselves; this module only dresses the `print!`
//! lines the CLI writes directly.
//!
//! Hand-rolled ANSI rather than ariadne's `Color`: ariadne's `Fmt` emits codes
//! unconditionally (it leaves the terminal check to the `Report` renderer), so
//! reusing it would mean either driving yansi's global enable state or always
//! allocating a styled wrapper. A handful of SGR constants gate explicitly, once
//! per stream, and hand back the input unchanged on the plain path.
//!
//! Gating (both must hold for a stream to be colored): the stream is a terminal
//! (`IsTerminal`, checked on stdout and stderr separately) **and** `NO_COLOR` is
//! unset or empty (no-color.org — any non-empty value disables). A piped or
//! redirected stream stays byte-for-byte plain, which is what the e2e suite reads.

use std::borrow::Cow;
use std::io::IsTerminal;
use std::sync::OnceLock;

/// An ANSI SGR style as its parameter body: `"32"` is green, `"1;31"` bold red.
#[derive(Clone, Copy)]
pub struct Style(&'static str);

impl Style {
    pub const GREEN: Style = Style("32");
    pub const YELLOW: Style = Style("33");
    pub const CYAN: Style = Style("36");
    pub const BOLD: Style = Style("1");
    pub const DIM: Style = Style("2");
    pub const BOLD_RED: Style = Style("1;31");
    pub const BOLD_GREEN: Style = Style("1;32");
    pub const BOLD_YELLOW: Style = Style("1;33");
}

/// Wraps `text` in `style`'s SGR codes when `enabled`; otherwise hands it back
/// borrowed and byte-identical — the plain path allocates nothing and never
/// reformats. Kept pure (the flag is a parameter) so the pins exercise both arms
/// without a real terminal.
fn wrap(enabled: bool, style: Style, text: &str) -> Cow<'_, str> {
    if enabled {
        Cow::Owned(format!("\x1b[{}m{}\x1b[0m", style.0, text))
    } else {
        Cow::Borrowed(text)
    }
}

/// The color gate for one stream: paint only a real terminal, and only when
/// `NO_COLOR` permits it. Both inputs are parameters, so the rule is pinned off a
/// TTY — and `NO_COLOR` winning over a terminal is just `is_terminal && !no_color`.
fn gate(is_terminal: bool, no_color: bool) -> bool {
    is_terminal && !no_color
}

/// `NO_COLOR` is honored when present and non-empty (no-color.org).
fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

// Each stream's verdict is computed once, on first paint — a build or watch run
// dresses many lines but probes the terminal (and `NO_COLOR`) a single time.
fn stdout_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| gate(std::io::stdout().is_terminal(), no_color()))
}

fn stderr_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| gate(std::io::stderr().is_terminal(), no_color()))
}

/// Paint `text` for a stdout line (`println!`), gated on stdout being a terminal.
pub fn out(style: Style, text: &str) -> Cow<'_, str> {
    wrap(stdout_enabled(), style, text)
}

/// Paint `text` for a stderr line (`eprintln!`), gated on stderr being a terminal.
pub fn err(style: Style, text: &str) -> Cow<'_, str> {
    wrap(stderr_enabled(), style, text)
}

/// The shared `error:` prefix (red + bold on a terminal, the plain literal when
/// piped) that opens every CLI error line outside the ariadne diagnostic path.
pub fn error_prefix() -> Cow<'static, str> {
    err(Style::BOLD_RED, "error:")
}

/// The shared `warning:` prefix (yellow + bold on a terminal). Bold for parity
/// with `error:` — a colored prefix reads as one unit.
pub fn warning_prefix() -> Cow<'static, str> {
    err(Style::BOLD_YELLOW, "warning:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_enabled_stream_paints_with_the_style_code() {
        assert_eq!(wrap(true, Style::GREEN, "ok"), "\x1b[32mok\x1b[0m");
    }

    #[test]
    fn a_disabled_stream_is_zero_alloc_byte_identical_passthrough() {
        let painted = wrap(false, Style::GREEN, "Compiled a -> b");
        assert_eq!(painted, "Compiled a -> b");
        // Borrowed, not a fresh String: the plain path costs no allocation and
        // hands back the exact bytes a pipe must see.
        assert!(matches!(painted, Cow::Borrowed(_)));
    }

    #[test]
    fn no_color_wins_over_a_terminal() {
        // The gate: NO_COLOR set (true) beats a terminal (true) → no color.
        assert!(!gate(true, true));
        // ...and end to end, the string comes out plain.
        assert_eq!(wrap(gate(true, true), Style::BOLD_RED, "error:"), "error:");
    }

    #[test]
    fn a_terminal_without_no_color_paints() {
        assert!(gate(true, false));
    }

    #[test]
    fn a_non_terminal_never_paints() {
        // Piped/redirected: plain regardless of NO_COLOR.
        assert!(!gate(false, false));
        assert!(!gate(false, true));
    }

    #[test]
    fn a_bold_colored_prefix_composes_both_sgr_codes() {
        // The error/warning prefixes lean on two-parameter styles.
        assert_eq!(
            wrap(true, Style::BOLD_RED, "error:"),
            "\x1b[1;31merror:\x1b[0m"
        );
        assert_eq!(
            wrap(true, Style::BOLD_YELLOW, "warning:"),
            "\x1b[1;33mwarning:\x1b[0m"
        );
    }
}
