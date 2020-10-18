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
    vol::{ReadVol},
    terrain::{SpriteKind},
};
use specs::{ Join, WorldExt };
use crate::display::Display;

use crate::{
    comp::{humanoid, Body},
};

fn main() {
    let screen_size = Vec2::new(80, 25);
    let view_distance = 12;
    let tps = 60;

    let matches = App::new("Teloren")
        .version("0.1")
        .author("Joshua Barretto <joshua.s.barretto@gmail.com>")
        .about("A terminal Veloren client frontend")
        .arg(Arg::with_name("username")
            .long("username")
            .value_name("USERNAME")
            .help("Set the username used to log in")
            .takes_value(true))
        .arg(Arg::with_name("password")
            .long("password")
            .value_name("PASSWORD")
            .help("Set the password to log in with")
            .takes_value(true))
        .arg(Arg::with_name("server")
            .long("server")
            .value_name("SERVER_ADDR")
            .help("Set the server address")
            .takes_value(true))
        .arg(Arg::with_name("port")
            .long("port")
            .value_name("PORT")
            .help("Set the server port")
            .takes_value(true))
        .arg(Arg::with_name("character")
            .long("character")
            .value_name("CHARACTER")
            .help("Select the character to play")
            .required(true)
            .takes_value(true))
        .get_matches();

    // Find arguments
    let server_addr = matches.value_of("server").unwrap_or("server.veloren.net");
    let server_port = matches.value_of("port").unwrap_or("14004");
    let username = matches.value_of("username").unwrap_or("teloren_user");
    let password = matches.value_of("password").unwrap_or("");
    let character_name = matches.value_of("character").unwrap_or("");
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

    client.register(username.to_string(), password.to_string(), |provider| provider == "https://auth.veloren.net")
        .unwrap_or_else(|err| {
            println!("Failed to register: {:?}", err);
            process::exit(1);
        });

    // Request character
    let clock = Clock::start();
    client.load_character_list();
    while client.active_character_id.is_none() {
        assert!(client.tick(comp::ControllerInputs::default(), clock.get_last_delta(), |_| ()).is_ok());
        if client.character_list.characters.len() > 0 {
            let character = client.character_list.characters.iter().find(|x| x.character.alias == character_name);
            if character.is_some() {
                let character_id = character.unwrap().character.id.unwrap();
                client.request_character(character_id);
                break;
            } 
            else {
                panic!("Character name not found!");
            }
        }
    }

    client.set_view_distance(view_distance);

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
                TermEvent::Key(Key::Char('w')) => inputs.move_dir.y += 1.0,
                TermEvent::Key(Key::Char('a')) => inputs.move_dir.x -= 1.0,
                TermEvent::Key(Key::Char('s')) => inputs.move_dir.y -= 1.0,
                TermEvent::Key(Key::Char('d')) => inputs.move_dir.x += 1.0,
                TermEvent::Mouse(me) => match me {
                    MouseEvent::Press(_, x, y) => tgt_pos = Some(from_screen_pos(Vec2::new(x, y), zoom_level)),
                    _ => {},
                },
                TermEvent::Key(Key::Char(' ')) => inputs.jump.set_state(true),
                TermEvent::Key(Key::Char('x')) => inputs.primary.set_state(true),
                TermEvent::Key(Key::Char('g')) => do_glide = !do_glide,
                TermEvent::Key(Key::Char('r')) => client.respawn(),
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
                Event::Chat(msg) => chat_log.push(msg.message),
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
                    let mut block_char = None;

                    for (k, z) in (-2..16).enumerate() {
                        block_z = wpos.z - z;

                        if let Some(b) = state
                            .terrain()
                            .get(wpos + Vec3::unit_z() * -z)
                            .ok()
                        {
                            let sprite = b.get_sprite();
                            if sprite.is_some() && sprite.unwrap()!=SpriteKind::Empty {
                                let sprite2 = sprite.unwrap();
                                let flower1 = SpriteKind::BarrelCactus as u8 ..= SpriteKind::Turnip as u8;
                                let flower2 = SpriteKind::LargeGrass as u8 ..= SpriteKind::LargeCactus as u8;
                                let furniture = SpriteKind::Window1 as u8 ..= SpriteKind::WardrobeDouble as u8;
                                block_char = match sprite2 {
                                    SpriteKind::Apple => Some('a'),
                                    SpriteKind::Sunflower => Some('u'),
                                    SpriteKind::Mushroom => Some('m'),
                                    SpriteKind::Velorite|SpriteKind::VeloriteFrag => Some('v'),
                                    SpriteKind::Chest|SpriteKind::Crate => Some('c'),
                                    SpriteKind::Stones => Some('s'),
                                    SpriteKind::Twigs => Some('t'),
                                    SpriteKind::ShinyGem => Some('g'),
                                    SpriteKind::Beehive => Some('b'),
                                    _ => {
                                        let sprite3 = sprite2 as u8;
                                        if flower1.contains(&sprite3) || flower2.contains(&sprite3)
                                        { Some('%') }
                                        else if furniture.contains(&sprite3) { Some('&') }
                                        else { None }
                                    }
                                };
                            }
                            else if b.is_filled() {
                                block = Some(*b);
                                if block_char.is_none() {
                                    block_char = Some(if k < level_chars.len() {
                                        level_chars[k as usize]
                                    } else {
                                        if block_z % 2 == 0 { 'O' } else { '0' }
                                    });
                                }
                                break;
                            }
                        }
                    }

                    let col = match block {
                        Some(block) => match block {
                            block if block.is_fluid() => Rgb::one(),
                            _ => block.get_color().unwrap_or(Rgb::one()),
                        },
                        None => Rgb::new(0, 255, 255),
                    };

                    if block_char.is_none() { block_char= Some('?'); }

                    write!(display, "{}{}", color::Rgb(col.r, col.g, col.b).fg_string(), block_char.unwrap()).unwrap();
                }
            }

            let objs = state.ecs().entities();
            let positions = state.ecs().read_storage::<comp::Pos>();
            let bodies = state.ecs().read_storage::<comp::Body>();

            for o in objs.join() {
                let pos = positions.get(o);
                let body = bodies.get(o);
                if pos.is_some() && body.is_some() {
                    let scr_pos = to_screen_pos(Vec2::from(pos.unwrap().0), zoom_level);
                    let character = 
                        match body.unwrap() {
                            Body::Humanoid(humanoid) => match humanoid.species {
                                humanoid::Species::Danari => 'R',
                                humanoid::Species::Dwarf => 'D',
                                humanoid::Species::Elf => 'E',
                                humanoid::Species::Human => 'H',
                                humanoid::Species::Orc => 'O',
                                humanoid::Species::Undead => 'U',
                            },
                            Body::QuadrupedLow(_) => '4',
                            Body::QuadrupedSmall(_) => 'q',
                            Body::QuadrupedMedium(_) => 'Q',
                            Body::BirdSmall(_) => 'b',
                            Body::BirdMedium(_) => 'B',
                            Body::FishSmall(_) => 'f',
                            Body::FishMedium(_) => 'F',
                            Body::BipedLarge(_) => '2',
                            Body::Object(_) => 'o',
                            Body::Golem(_) => 'G',
                            Body::Dragon(_) => 'S',
                            Body::Theropod(_) => 'T',
                            _ => '?'
                    };

                    if scr_pos
                        .map2(screen_size, |e, sz| e >= 0 && e < sz as i32)
                        .reduce_and()
                    {
                        write!(display.at((scr_pos.x as u16, scr_pos.y as u16)), "{}{}", color::White.fg_str(), character).unwrap();
                    }
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
