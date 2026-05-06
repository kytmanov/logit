use std::io::IsTerminal;

#[derive(Debug, Clone, Copy)]
pub struct Style {
    pub color: bool,
    pub unicode: bool,
}

impl Style {
    pub fn for_stdout() -> Self {
        Self {
            color: color_enabled(std::io::stdout().is_terminal()),
            unicode: unicode_enabled(),
        }
    }

    pub fn for_stderr() -> Self {
        Self {
            color: color_enabled(std::io::stderr().is_terminal()),
            unicode: unicode_enabled(),
        }
    }

    pub fn plain() -> Self {
        Self {
            color: false,
            unicode: false,
        }
    }

    pub fn bold(&self, text: &str) -> String {
        self.wrap(text, "\x1b[1m")
    }

    pub fn dim(&self, text: &str) -> String {
        self.wrap(text, "\x1b[2m")
    }

    pub fn green(&self, text: &str) -> String {
        self.wrap(text, "\x1b[32m")
    }

    pub fn yellow(&self, text: &str) -> String {
        self.wrap(text, "\x1b[33m")
    }

    pub fn red(&self, text: &str) -> String {
        self.wrap(text, "\x1b[31m")
    }

    pub fn cyan(&self, text: &str) -> String {
        self.wrap(text, "\x1b[36m")
    }

    pub fn bold_cyan(&self, text: &str) -> String {
        self.wrap(text, "\x1b[1;36m")
    }

    fn wrap(&self, text: &str, prefix: &str) -> String {
        if self.color {
            format!("{prefix}{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    pub fn check(&self) -> &'static str {
        if self.unicode { "✓" } else { "[OK]" }
    }

    pub fn cross(&self) -> &'static str {
        if self.unicode { "✗" } else { "[X]" }
    }

    pub fn warn(&self) -> &'static str {
        if self.unicode { "⚠" } else { "[!]" }
    }

    pub fn arrow(&self) -> &'static str {
        if self.unicode { "→" } else { "->" }
    }

    pub fn dot(&self) -> &'static str {
        if self.unicode { "·" } else { "-" }
    }

    pub fn dry_run_glyph(&self) -> &'static str {
        if self.unicode { "↻" } else { "[~]" }
    }

    pub fn dash(&self) -> &'static str {
        if self.unicode { "—" } else { "-" }
    }
}

fn color_enabled(is_tty: bool) -> bool {
    if !is_tty {
        return false;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var_os("LOGIT_NO_COLOR").is_some() {
        return false;
    }
    if matches!(std::env::var("TERM").as_deref(), Ok("dumb")) {
        return false;
    }
    true
}

fn unicode_enabled() -> bool {
    if std::env::var_os("LOGIT_ASCII").is_some() {
        return false;
    }
    if matches!(std::env::var("TERM").as_deref(), Ok("dumb")) {
        return false;
    }
    if cfg!(unix) {
        let lang = std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LC_CTYPE"))
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_default();
        if lang.is_empty() {
            return false;
        }
        let upper = lang.to_uppercase();
        return upper.contains("UTF-8") || upper.contains("UTF8");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_style_strips_ansi() {
        let style = Style::plain();
        assert_eq!(style.bold("x"), "x");
        assert_eq!(style.green("y"), "y");
        assert_eq!(style.check(), "[OK]");
    }

    #[test]
    fn color_style_wraps_ansi() {
        let style = Style {
            color: true,
            unicode: true,
        };
        assert_eq!(style.bold("x"), "\x1b[1mx\x1b[0m");
        assert_eq!(style.check(), "✓");
    }
}
