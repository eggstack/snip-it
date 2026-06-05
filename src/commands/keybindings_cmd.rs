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
    Ok(())
}
