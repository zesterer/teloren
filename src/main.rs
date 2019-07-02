use std::{
    sync::mpsc,
    thread,
    io::{Write, stdin, stdout},
};
use vek::*;
use terminal_graphics::{
    Display,
    graphics::Graphics,
};
use termion::{
    raw::IntoRawMode,
    input::TermRead,
    event::Key,
    cursor,
};

const SCREEN_SIZE: Vec2<u32> = Vec2 {
    x: 80,
    y: 25,
};

fn main() {
    let mut screen = Display::new(SCREEN_SIZE.x, SCREEN_SIZE.y);
    screen.clear();

    let mut stdout = stdout().into_raw_mode().unwrap();

    let stdin = stdin();
    let (key_tx, key_rx) = mpsc::channel();
    thread::spawn(move || {
        stdin.lock();
        for c in stdin.keys() {
            key_tx.send(c).unwrap();
        }
    });

    let mut graphics = Graphics::new();

    write!(stdout, "{}", cursor::Hide);

    'running: loop {
        for c in key_rx.try_iter() {
            match c.unwrap() {
                Key::Char('q') => break 'running,
                _ => {},
            }
        }

        graphics.draw(&mut screen);

        stdout.flush().unwrap();
    }
}
