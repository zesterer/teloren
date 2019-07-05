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
    event::{Event, Key, MouseEvent},
    color,
};
use clap::{App, Arg};
use veloren_client::Client;
use veloren_common::{
    comp,
    clock::Clock,
    vol::{ReadVol, Vox},
    terrain::TerrainMap,
};
use specs::Join;
use crate::display::Display;

fn main() {
    let screen_size = Vec2::new(80, 25);
    let view_distance = 2;
    let tps = 30;

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
    let server_port = matches.value_of("port").unwrap_or("59003");
    let alias = matches.value_of("alias").unwrap_or("teloren_user");

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

    client.register(comp::Player::new(alias.to_string(), Some(view_distance)));

    client.request_character(alias.to_string(), comp::Body::Humanoid(comp::humanoid::Body::random()));

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
    let mut tgt_pos = None;

    'running: for _ in 0.. {
        // Get player pos
        let player_pos = client
            .state()
            .ecs()
            .read_storage::<comp::Pos>()
            .get(client.entity())
            .map(|pos| pos.0.map(|e| e.floor() as i32))
            .unwrap_or(Vec3::zero());

        let to_screen_pos = |pos: Vec2<f32>|
            pos.map(|e| e as i32)
            - Vec2::from(player_pos)
            + screen_size.map(|e| e as i32) / 2;

        let from_screen_pos = |pos: Vec2<u16>|
            Vec2::from(player_pos)
            + pos.map(|e| e as i32)
            - screen_size.map(|e| e as i32) / 2;

        let mut controller = comp::Controller::default();

        // Handle inputs
        for c in key_rx.try_iter() {
            match c.unwrap() {
                Event::Key(Key::Char('w')) => controller.move_dir.y -= 1.0,
                Event::Key(Key::Char('a')) => controller.move_dir.x -= 1.0,
                Event::Key(Key::Char('s')) => controller.move_dir.y += 1.0,
                Event::Key(Key::Char('d')) => controller.move_dir.x += 1.0,
                Event::Mouse(me) => match me {
                    MouseEvent::Press(_, x, y) => tgt_pos = Some(from_screen_pos(Vec2::new(x, y))),
                    _ => {},
                },
                Event::Key(Key::Char(' ')) => controller.jump = true,
                Event::Key(Key::Char('x')) => controller.attack = true,
                Event::Key(Key::Char('g')) => do_glide = !do_glide,
                Event::Key(Key::Char('r')) => controller.respawn = true,
                Event::Key(Key::Char('q')) => break 'running,
                _ => {},
            }
        }

        if do_glide {
            controller.glide = true;
        }
        if let Some(tp) = tgt_pos {
            if tp.distance_squared(player_pos.into()) < 1 {
                tgt_pos = None;
            } else {
                controller.move_dir = (tp - Vec2::from(player_pos)).map(|e: i32| e as f32).normalized();
            }
        }

        // Tick client
        client.tick(controller, clock.get_last_delta()).unwrap();
        client.cleanup();

        let ecs = client.state().ecs();

        // Render block
        for j in 0..screen_size.y {
            let mut display = display.at((0, j));

            for i in 0..screen_size.x {
                let wpos = player_pos + Vec3::new(i, j, 0)
                    .map2(screen_size.into(), |e, sz: u16| e as i32 - sz as i32 / 2);

                let mut block_z = 0;
                let mut block = None;

                for z in -2..16 {
                    block_z = wpos.z - z;

                    if let Some(b) = ecs
                        .read_resource::<TerrainMap>()
                        .get(wpos + Vec3::unit_z() * -z)
                        .ok()
                        .filter(|b| !b.is_empty())
                    {
                        block = Some(*b);
                        break;
                    }
                }

                let (col, c) = match block {
                    Some(block) => match block {
                        block if block.is_empty() => (Rgb::one(), ' '),
                        _ => (block.get_color().unwrap_or(Rgb::one()), if block_z % 2 == 0 { '#' } else { '%' }),
                    },
                    None => (Rgb::new(0, 255, 255), '?'),
                };

                write!(display, "{}{}", color::Rgb(col.r, col.g, col.b).fg_string(), c).unwrap();
            }
        }

        for (
            pos,
        ) in (
            &ecs.read_storage::<comp::Pos>(),
        ).join() {
            let scr_pos = to_screen_pos(Vec2::from(pos.0));

            if scr_pos
                .map2(screen_size, |e, sz| e >= 0 && e < sz as i32)
                .reduce_and()
            {
                write!(display.at((scr_pos.x as u16, scr_pos.y as u16)), "{}{}", color::White.fg_str(), '@').unwrap();
            }
        }

        write!(display.at((0, screen_size.y + 0)), "======= Controls =======").unwrap();
        write!(display.at((0, screen_size.y + 1)), "|  wasd - Move         |").unwrap();
        write!(display.at((0, screen_size.y + 2)), "| click - Move         |").unwrap();
        write!(display.at((0, screen_size.y + 3)), "| space - Jump         |").unwrap();
        write!(display.at((0, screen_size.y + 4)), "|     x - Attack       |").unwrap();
        if do_glide {
        write!(display.at((0, screen_size.y + 5)), "|     g - Stop gliding |").unwrap();
        } else {
        write!(display.at((0, screen_size.y + 5)), "|     g - Glide        |").unwrap();
        }
        write!(display.at((0, screen_size.y + 6)), "|     r - Respawn      |").unwrap();
        write!(display.at((0, screen_size.y + 7)), "|     q - Quit         |").unwrap();
        write!(display.at((0, screen_size.y + 8)), "========================").unwrap();

        // Finish drawing
        display.flush();

        // Wait for next tick
        clock.tick(Duration::from_millis(1000 / tps));
    }
}
