use crate::error::SnipResult;

/// Prints all available TUI keybindings to stdout.
pub fn run() -> SnipResult<()> {
    println!("Current keybindings:");
    println!();
    println!("Normal Mode:");
    println!("  h/←   : move left");
    println!("  j/↓   : move down");
    println!("  k/↑   : move up");
    println!("  l/→   : move right");
    println!("  gg    : jump to top (Ctrl+g)");
    println!("  G     : jump to bottom");
    println!("  Ctrl+f: page down");
    println!("  Ctrl+d: page down (helix)");
    println!("  Ctrl+b: page up");
    println!("  Ctrl+u: page up (helix)");
    println!("  v     : visual mode (character)");
    println!("  V     : visual mode (line)");
    println!("  y     : copy and quit");
    println!("  p     : preview command");
    println!("  i     : insert mode");
    println!("  e     : open theme picker");
    println!("  q     : quit");
    println!("  Esc   : no-op (use q to quit)");
    println!("  /     : search");
    println!("  t     : toggle tag filter");
    println!("  n     : sort by newest");
    println!("  o     : sort by oldest");
    println!("  a     : sort a-z");
    println!("  z     : sort z-a");
    println!("  d     : clear filter");
    println!();
    println!("Insert Mode:");
    println!("  j/k   : alternative navigation");
    println!("  ↑/↓   : move up/down");
    println!("  Enter : select/execute");
    println!("  Esc   : return to normal mode");
    println!("  /     : start search");
    println!("  Backspace: delete character");
    println!();
    println!("Theme Picker (opened with e in normal mode):");
    println!("  i     : filter (insert mode)");
    println!("  j/↓   : next theme (live preview)");
    println!("  k/↑   : previous theme (live preview)");
    println!("  Ctrl+d/PageDown : page down (10 themes)");
    println!("  Ctrl+u/PageUp   : page up (10 themes)");
    println!("  gg    : first theme");
    println!("  G     : last theme");
    println!("  Enter : save & apply theme");
    println!("  e/q   : cancel & revert to previous theme");
    println!("  Esc   : leave filter (back to picker normal mode)");
    Ok(())
}
