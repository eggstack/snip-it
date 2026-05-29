use std::sync::LazyLock;

use ratatui::style::{Color, Style};

#[derive(Clone, Copy)]
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub background: Color,
    pub text: Color,
    pub border: Color,
    pub selected_bg: Color,
    pub muted: Color,
}

const DARK_THEME: Theme = Theme {
    primary: Color::Blue,
    secondary: Color::Cyan,
    accent: Color::Yellow,
    background: Color::Black,
    text: Color::White,
    border: Color::Cyan,
    selected_bg: Color::Blue,
    muted: Color::Gray,
};

const BRIGHT_THEME: Theme = Theme {
    primary: Color::Blue,
    secondary: Color::Blue,
    accent: Color::Magenta,
    background: Color::White,
    text: Color::Black,
    border: Color::Blue,
    selected_bg: Color::LightBlue,
    muted: Color::Gray,
};

fn resolve_theme(theme_name: &str) -> Theme {
    match theme_name {
        "bright" | "light" => BRIGHT_THEME,
        "dark" => DARK_THEME,
        "auto" => {
            if std::env::var("COLORFGBG")
                .map(|v| v.starts_with("15;") || v.starts_with("7;"))
                .unwrap_or(false)
            {
                BRIGHT_THEME
            } else {
                DARK_THEME
            }
        }
        _ => DARK_THEME,
    }
}

static ACTIVE_THEME: LazyLock<std::sync::Mutex<Theme>> = LazyLock::new(|| {
    std::sync::Mutex::new({
        let theme_name = std::env::var("SNP_THEME").unwrap_or_else(|_| "auto".to_string());
        resolve_theme(&theme_name)
    })
});

pub fn get_theme() -> std::sync::MutexGuard<'static, Theme> {
    ACTIVE_THEME.lock().unwrap()
}

pub(crate) fn style_fg(fg: Color) -> Style {
    Style::default().fg(fg)
}

pub(crate) fn style_fg_bg(fg: Color, bg: Color) -> Style {
    Style::default().fg(fg).bg(bg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_theme_dark() {
        let theme = resolve_theme("dark");
        assert_eq!(theme.background, Color::Black);
    }

    #[test]
    fn test_resolve_theme_bright() {
        let theme = resolve_theme("bright");
        assert_eq!(theme.background, Color::White);
    }

    #[test]
    fn test_resolve_theme_light() {
        let theme = resolve_theme("light");
        assert_eq!(theme.background, Color::White);
    }

    #[test]
    fn test_resolve_theme_unknown() {
        let theme = resolve_theme("unknown");
        assert_eq!(theme.background, Color::Black);
    }
}
