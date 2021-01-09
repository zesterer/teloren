use std::io::{self, Stdout, Write};
use termion::{
    clear, cursor,
    input::MouseTerminal,
    raw::{IntoRawMode, RawTerminal},
};
use vek::*;

pub struct Display {
    size: Vec2<u16>,
    stdout: MouseTerminal<RawTerminal<Stdout>>,
}

impl Display {
    pub fn new(size: impl Into<Vec2<u16>>, stdout: Stdout) -> Self {
        let mut this = Self {
            size: size.into(),
            stdout: MouseTerminal::from(stdout.into_raw_mode().unwrap()),
        };
        this.init();
        this
    }

    fn init(&mut self) {
        write!(self.stdout, "{}{}", clear::All, cursor::Hide).unwrap();
    }

    pub fn clear_with(&mut self, c: char) {
        for y in 1..=self.size.y {
            write!(self.stdout, "{}", cursor::Goto(1, y + 1)).unwrap();
            for _ in 1..=self.size.x {
                write!(self.stdout, "{}", c).unwrap();
            }
        }
    }

    pub fn at(&mut self, pos: impl Into<Vec2<u16>>) -> DisplayAt {
        let pos = pos.into();
        write!(self.stdout, "{}", cursor::Goto(pos.x + 1, pos.y + 1)).unwrap();
        DisplayAt(self)
    }

    pub fn flush(&mut self) {
        self.stdout.flush().unwrap();
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        write!(self.stdout, "{}{}", clear::All, cursor::Show).unwrap();
    }
}

pub struct DisplayAt<'a>(&'a mut Display);

impl<'a> io::Write for DisplayAt<'a> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.stdout.write(b)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.stdout.flush()
    }
}
