mod display;

use std::{
    sync::mpsc,
    thread,
    io::{stdin, stdout, Write},
    net::ToSocketAddrs,
    time::Duration,
    process,
};
use vek::*;
use termion::{
    input::TermRead,
    event::{Event as TermEvent, Key, MouseEvent},
    color,
};
use clap::{App, Arg};
use veloren_client::{Client, Event};
use veloren_common::{
    comp,
    clock::Clock,
    vol::{ReadVol, Vox},
};
use specs::Join;
use crate::display::Display;

fn main() {
    let screen_size = Vec2::new(80, 25);
    let view_distance = 12;
    let tps = 60;

    let matches = App::new("Teloren")
        .version("0.1")
        .author("Joshua Barretto <joshua.s.barretto@gmail.com>")
        .about("A terminal Veloren client frontend")
        .arg(Arg::with_name("alias")
            .short("a")
            .long("alias")
            .value_name("ALIAS")
            .help("Set the in-game alias")
            .takes_value(true))
        .arg(Arg::with_name("server")
            .short("s")
            .long("server")
            .value_name("SERVER_ADDR")
            .help("Set the server address")
            .takes_value(true))
        .arg(Arg::with_name("port")
            .short("p")
            .long("port")
            .value_name("PORT")
            .help("Set the server port")
            .takes_value(true))
        .get_matches();

    // Find arguments
    let server_addr = matches.value_of("server").unwrap_or("server.veloren.net");
    let server_port = matches.value_of("port").unwrap_or("14004");
    let alias = matches.value_of("alias").unwrap_or("teloren_user");
    //let password = matches.value_of("password").unwrap_or("");

    // Parse server socket
    let server_sock = format!("{}:{}", server_addr, server_port)
        .to_socket_addrs()
        .unwrap_or_else(|_| {
            println!("Invalid server address");
            process::exit(1);
        })
        .next()
        .unwrap();

    let mut client = Client::new(server_sock, Some(view_distance))
        .unwrap_or_else(|err| {
            println!("Failed to connect to server: {:?}", err);
            process::exit(1);
        });

    println!("Server info: {:?}", client.server_info);
    println!("Players: {:?}", client.get_players());

    client.register(comp::Player::new(alias.to_string(), Some(view_distance)), "".to_string())
        .unwrap_or_else(|err| {
            println!("Failed to register player: {:?}", err);
            process::exit(1);
        });

    client.request_character(alias.to_string(), comp::Body::Humanoid(comp::humanoid::Body::random()), None);

    // Spawn input thread
    let stdin = stdin();
    let (key_tx, key_rx) = mpsc::channel();
    thread::spawn(move || {
        stdin.lock();
        for c in stdin.events() {
            key_tx.send(c).unwrap();
        }
    });

    let mut display = Display::new(screen_size, stdout());
    let mut clock = Clock::start();
    let mut do_glide = false;
    let mut zoom_level = 1.0;
    let mut tgt_pos = None;
    let mut chat_log = Vec::new();
    let mut chat_input = String::new();
    let mut chat_input_enabled = false;

    'running: for tick in 0.. {
        // Get player pos
        let player_pos = client
            .state()
            .read_storage::<comp::Pos>()
            .get(client.entity())
            .map(|pos| pos.0)
            .unwrap_or(Vec3::zero());

        let to_screen_pos = |pos: Vec2<f32>, zoom_level: f32|
            ((pos
            - Vec2::from(player_pos)) * Vec2::new(1.0, -1.0) / zoom_level
            + screen_size.map(|e| e as f32) / 2.0)
            .map(|e| e as i32);

        let from_screen_pos = |pos: Vec2<u16>, zoom_level: f32|
            Vec2::from(player_pos)
            + (pos.map(|e| e as f32)
            - screen_size.map(|e| e as f32) / 2.0) * zoom_level * Vec2::new(1.0, -1.0);

        let mut inputs = comp::ControllerInputs::default();

        // Handle inputs
        for c in key_rx.try_iter() {
            match c.unwrap() {
                TermEvent::Key(Key::Char(c)) if chat_input_enabled => match c {
                    '\n' => {
                        client.send_chat(chat_input.clone());
                        chat_input = String::new();
                        chat_input_enabled = false;
                    },
                    '\x08' => { chat_input.pop(); },
                    c => chat_input.push(c),
                },
                TermEvent::Key(Key::Char('\n')) => chat_input_enabled = true,
                TermEvent::Key(Key::Char('w')) => inputs.move_dir.y -= 1.0,
                TermEvent::Key(Key::Char('a')) => inputs.move_dir.x -= 1.0,
                TermEvent::Key(Key::Char('s')) => inputs.move_dir.y += 1.0,
                TermEvent::Key(Key::Char('d')) => inputs.move_dir.x += 1.0,
                TermEvent::Mouse(me) => match me {
                    MouseEvent::Press(_, x, y) => tgt_pos = Some(from_screen_pos(Vec2::new(x, y), zoom_level)),
                    _ => {},
                },
                TermEvent::Key(Key::Char(' ')) => inputs.jump.set_state(true),
                TermEvent::Key(Key::Char('x')) => inputs.primary.set_state(true),
                TermEvent::Key(Key::Char('g')) => do_glide = !do_glide,
                TermEvent::Key(Key::Char('r')) => inputs.respawn.set_state(true),
                TermEvent::Key(Key::Char('+')) => zoom_level /= 1.5,
                TermEvent::Key(Key::Char('-')) => zoom_level *= 1.5,
                TermEvent::Key(Key::Char('q')) => break 'running,
                _ => {},
            }
        }

        if do_glide {
            inputs.glide.set_state(true);
        }
        if let Some(tp) = tgt_pos {
            if tp.distance_squared(player_pos.into()) < 1.0 {
                tgt_pos = None;
            } else {
                inputs.move_dir = (tp - Vec2::from(player_pos)).try_normalized().unwrap_or(Vec2::zero());
            }
        }

        // Tick client
        for event in client.tick(inputs, clock.get_last_delta(), |_| ()).unwrap() {
            match event {
                Event::Chat { message, .. } => chat_log.push(message),
                _ => {},
            }
        }
        client.cleanup();

        // Drawing
        if tick % 6 == 0 {
            let state = client.state();

            let level_chars = ['#', '+', '='];

            // Render block
            for j in 0..screen_size.y {
                let mut display = display.at((0, j));

                for i in 0..screen_size.x {
                    let wpos = (player_pos + Vec3::new(i, j, 0)
                        .map2(screen_size.into(), |e, sz: u16| e as f32 - sz as f32 / 2.0)
                        * Vec2::new(1.0, -1.0) * zoom_level)
                        .map(|e| e.floor() as i32);

                    let mut block_z = 0;
                    let mut block = None;
                    let mut block_char = '?';

                    for (k, z) in (-2..16).enumerate() {
                        block_z = wpos.z - z;

                        if let Some(b) = state
                            .terrain()
                            .get(wpos + Vec3::unit_z() * -z)
                            .ok()
                            .filter(|b| !b.is_empty())
                        {
                            block = Some(*b);
                            block_char = if k < level_chars.len() {
                                level_chars[k as usize]
                            } else {
                                if block_z % 2 == 0 { 'O' } else { '0' }
                            };
                            break;
                        }
                    }

                    let col = match block {
                        Some(block) => match block {
                            block if block.is_empty() => Rgb::one(),
                            _ => block.get_color().unwrap_or(Rgb::one()),
                        },
                        None => Rgb::new(0, 255, 255),
                    };

                    write!(display, "{}{}", color::Rgb(col.r, col.g, col.b).fg_string(), block_char).unwrap();
                }
            }

            for pos in state.read_storage::<comp::Pos>().join() {
                let scr_pos = to_screen_pos(Vec2::from(pos.0), zoom_level);

                if scr_pos
                    .map2(screen_size, |e, sz| e >= 0 && e < sz as i32)
                    .reduce_and()
                {
                    write!(display.at((scr_pos.x as u16, scr_pos.y as u16)), "{}{}", color::White.fg_str(), '@').unwrap();
                }
            }

            write!(display.at((0, screen_size.y + 0)), "/------- Controls ------\\").unwrap();
            write!(display.at((0, screen_size.y + 1)), "|   wasd - Move         |").unwrap();
            write!(display.at((0, screen_size.y + 2)), "|  click - Move         |").unwrap();
            write!(display.at((0, screen_size.y + 3)), "|  space - Jump         |").unwrap();
            write!(display.at((0, screen_size.y + 4)), "|      x - Attack       |").unwrap();
            if do_glide {
            write!(display.at((0, screen_size.y + 5)), "|      g - Stop gliding |").unwrap();
            } else {
            write!(display.at((0, screen_size.y + 5)), "|      g - Glide        |").unwrap();
            }
            write!(display.at((0, screen_size.y + 6)), "|      r - Respawn      |").unwrap();
            write!(display.at((0, screen_size.y + 7)), "|      q - Quit         |").unwrap();
            write!(display.at((0, screen_size.y + 8)), "|      + - Zoom in      |").unwrap();
            write!(display.at((0, screen_size.y + 9)), "|      - - Zoom out     |").unwrap();
            write!(display.at((0, screen_size.y + 10)), "| return - Chat         |").unwrap();
            write!(display.at((0, screen_size.y + 11)), "\\-----------------------/").unwrap();

            let clear = "                                                                ";
            for (i, msg) in chat_log.iter().rev().take(10).enumerate() {
                write!(display.at((24, screen_size.y + 10 - i as u16)), "{}", clear).unwrap();
                write!(display.at((24, screen_size.y + 10 - i as u16)), "{}", msg.get(0..48).unwrap_or(&msg)).unwrap();
            }
            write!(display.at((24, screen_size.y + 11)), "{}", clear).unwrap();
            write!(display.at((24, screen_size.y + 11)), "> {}", chat_input).unwrap();
        }

        // Finish drawing
        display.flush();

        // Wait for next tick
        clock.tick(Duration::from_millis(1000 / tps));
    }
}
