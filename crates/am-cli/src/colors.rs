/// Terminal color support that respects `NO_COLOR`, `TERM=dumb`, and TTY detection.
///
/// Returns ANSI escape codes when color output is appropriate, empty strings otherwise.
pub(crate) struct Colors {
    pub bold: &'static str,
    pub dim: &'static str,
    pub reset: &'static str,
    pub cyan: &'static str,
    pub yellow: &'static str,
}

impl Colors {
    /// Detect whether stdout supports color output.
    ///
    /// Respects the `NO_COLOR` convention (<https://no-color.org/>),
    /// `TERM=dumb`, and whether stdout is a terminal.
    pub fn stdout() -> Self {
        if should_colorize() {
            Self::enabled()
        } else {
            Self::disabled()
        }
    }

    const fn enabled() -> Self {
        Self {
            bold: "\x1b[1m",
            dim: "\x1b[2m",
            reset: "\x1b[0m",
            cyan: "\x1b[36m",
            yellow: "\x1b[33m",
        }
    }

    const fn disabled() -> Self {
        Self {
            bold: "",
            dim: "",
            reset: "",
            cyan: "",
            yellow: "",
        }
    }
}

fn should_colorize() -> bool {
    use std::io::IsTerminal;

    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        return false;
    }
    std::io::stdout().is_terminal()
}
