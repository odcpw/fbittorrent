//! Every line this binary prints with `print!`/`println!`/`eprint!`/`eprintln!`
//! passes through `public_text::terminal_lines`. Torrent, feed, provider and
//! search names (and the URLs and error messages around them) are remote
//! text: a raw ESC/CSI/OSC sequence in one would drive the user's terminal
//! (clipboard writes, retitling, hidden or rewritten output). Newlines and
//! tabs pass. The TUI draws through its own filtered renderer and raw writers.
//! Declared first in main.rs, these shadow the std macros crate-wide.

#[allow(unused_macros)]
macro_rules! println {
    () => { ::std::println!() };
    ($($arg:tt)*) => { ::std::println!("{}", $crate::public_text::terminal_lines(&::std::format!($($arg)*))) };
}
#[allow(unused_macros)]
macro_rules! print {
    ($($arg:tt)*) => { ::std::print!("{}", $crate::public_text::terminal_lines(&::std::format!($($arg)*))) };
}
#[allow(unused_macros)]
macro_rules! eprintln {
    () => { ::std::eprintln!() };
    ($($arg:tt)*) => { ::std::eprintln!("{}", $crate::public_text::terminal_lines(&::std::format!($($arg)*))) };
}
#[allow(unused_macros)]
macro_rules! eprint {
    ($($arg:tt)*) => { ::std::eprint!("{}", $crate::public_text::terminal_lines(&::std::format!($($arg)*))) };
}
