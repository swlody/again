#[macro_use]
extern crate static_assertions;

mod game;

#[cfg(windows)]
mod win32;

use std::io;

fn main() -> io::Result<()> {
    #[cfg(windows)]
    win32::win32_main()?;

    Ok(())
}
