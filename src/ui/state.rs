use ratatui::widgets::ScrollbarState;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum SortMode {
    #[default]
    None,
    Newest,
    Oldest,
    AlphaAsc,
    AlphaDesc,
}

#[derive(Clone, Default)]
pub(super) struct SelectState {
    pub selected: usize,
    pub list_state: ratatui::widgets::ListState,
    pub scroll_state: ScrollbarState,
}

impl SelectState {
    pub fn new() -> Self {
        let mut list_state = ratatui::widgets::ListState::default();
        list_state.select(Some(0));
        SelectState {
            selected: 0,
            list_state,
            scroll_state: ScrollbarState::default(),
        }
    }

    pub fn update(&mut self, filtered_len: usize) {
        if filtered_len == 0 {
            self.selected = 0;
        } else if self.selected >= filtered_len {
            self.selected = filtered_len.saturating_sub(1);
        }
        self.list_state.select(Some(self.selected));
        self.scroll_state = self
            .scroll_state
            .content_length(filtered_len)
            .position(self.selected);
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self, max: usize) {
        if self.selected + 1 < max {
            self.selected += 1;
        }
    }

    pub fn move_to_top(&mut self) {
        self.selected = 0;
    }

    pub fn move_to_bottom(&mut self, max: usize) {
        self.selected = max.saturating_sub(1);
    }
}

#[derive(Clone, Default)]
pub(super) struct FilterState {
    pub sort_mode: SortMode,
    pub tag_filter_text: String,
}

impl FilterState {
    pub fn toggle_sort_new(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::Newest {
            SortMode::None
        } else {
            SortMode::Newest
        };
    }
    pub fn toggle_sort_old(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::Oldest {
            SortMode::None
        } else {
            SortMode::Oldest
        };
    }
    pub fn toggle_sort_alpha(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::AlphaAsc {
            SortMode::None
        } else {
            SortMode::AlphaAsc
        };
    }
    pub fn toggle_sort_alpha_rev(&mut self) {
        self.sort_mode = if self.sort_mode == SortMode::AlphaDesc {
            SortMode::None
        } else {
            SortMode::AlphaDesc
        };
    }
}

pub(super) fn is_ctrl_key(key: &crossterm::event::KeyEvent, c: char) -> bool {
    key.code == crossterm::event::KeyCode::Char(c)
        && key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_state_toggle_sort_new() {
        let mut state = FilterState::default();
        assert_eq!(state.sort_mode, SortMode::None);

        state.toggle_sort_new();
        assert_eq!(state.sort_mode, SortMode::Newest);

        state.toggle_sort_new();
        assert_eq!(state.sort_mode, SortMode::None);
    }

    #[test]
    fn test_filter_state_toggle_sort_alpha() {
        let mut state = FilterState::default();
        assert_eq!(state.sort_mode, SortMode::None);

        state.toggle_sort_alpha();
        assert_eq!(state.sort_mode, SortMode::AlphaAsc);

        state.toggle_sort_alpha_rev();
        assert_eq!(state.sort_mode, SortMode::AlphaDesc);
    }

    #[test]
    fn test_is_ctrl_key_true() {
        let key = crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char('c'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(is_ctrl_key(&key, 'c'));
    }

    #[test]
    fn test_is_ctrl_key_false_no_modifier() {
        let key = crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char('c'),
            modifiers: crossterm::event::KeyModifiers::empty(),
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(!is_ctrl_key(&key, 'c'));
    }

    #[test]
    fn test_is_ctrl_key_false_different_char() {
        let key = crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Char('a'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: crossterm::event::KeyEventKind::Press,
            state: crossterm::event::KeyEventState::NONE,
        };
        assert!(!is_ctrl_key(&key, 'c'));
    }
}
