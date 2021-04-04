mod display;

use crate::comp::{humanoid, Body};
use crate::display::Display;
use clap::{App, Arg};
use std::{
    io::{stdin, stdout, Write},
    process,
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};
use termion::{
    color,
    event::{Event as TermEvent, Key, MouseEvent},
    input::TermRead,
};
use tokio::runtime::Runtime;
use vek::*;
use veloren_client::{
    addr::ConnectionArgs, Client, Event, Join, Marker, MarkerAllocator, WorldExt,
};
use veloren_common::{
    clock::Clock,
    comp,
    comp::{Energy, Health, InputKind},
    terrain::SpriteKind,
    uid::UidAllocator,
    vol::ReadVol,
};

fn main() {
    let screen_size = Vec2::new(80, 25);
    let view_distance = 12;
    let tps = 60;
    let mut is_secondary_active: bool = false;
    let matches = App::new("Teloren")
        .version("0.1")
        .author("Joshua Barretto <joshua.s.barretto@gmail.com>")
        .about("A terminal Veloren client frontend")
        .arg(
            Arg::with_name("username")
                .long("username")
                .value_name("USERNAME")
                .help("Set the username used to log in")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("password")
                .long("password")
                .value_name("PASSWORD")
                .help("Set the password to log in with")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("server")
                .long("server")
                .value_name("SERVER_ADDR")
                .help("Set the server address")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("port")
                .long("port")
                .value_name("PORT")
                .help("Set the server port")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("character")
                .long("character")
                .value_name("CHARACTER")
                .help("Select the character to play")
                .required(true)
                .takes_value(true),
        )
        .get_matches();

    // Find arguments
    let server_addr = matches.value_of("server").unwrap_or("server.veloren.net");
    let server_port = matches.value_of("port").unwrap_or("14004");
    let username = matches.value_of("username").unwrap_or("teloren_user");
    let password = matches.value_of("password").unwrap_or("");
    let character_name = matches.value_of("character").unwrap_or("");

    // Parse server socket
    let server_spec = format!("{}:{}", server_addr, server_port);

    let runtime = Arc::new(Runtime::new().unwrap());
    let runtime2 = Arc::clone(&runtime);

    let mut client = runtime
        .block_on(async {
            let addr = ConnectionArgs::resolve(&server_spec, false)
                .await
                .expect("dns resolve failed");
            Client::new(addr, Some(view_distance), runtime2).await
        })
        .expect("Failed to create client instance");

    println!("Server info: {:?}", client.server_info());
    println!("Players: {:?}", client.get_players());

    runtime
        .block_on(
            client.register(username.to_string(), password.to_string(), |provider| {
                provider == "https://auth.veloren.net"
            }),
        )
        .unwrap_or_else(|err| {
            println!("Failed to register: {:?}", err);
            process::exit(1);
        });

    // Request character
    let mut clock = Clock::new(Duration::from_secs_f64(1.0 / tps as f64));
    client.load_character_list();

    while client.presence().is_none() {
        assert!(client
            .tick(comp::ControllerInputs::default(), clock.dt(), |_| ())
            .is_ok());
        if client.character_list().characters.len() > 0 {
            let character = client
                .character_list()
                .characters
                .iter()
                .find(|x| x.character.alias == character_name);
            if character.is_some() {
                let character_id = character.unwrap().character.id.unwrap();
                client.request_character(character_id);
                break;
            } else {
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
    let mut zoom_level = 1.0;
    let mut tgt_pos = None;
    let mut chat_log = Vec::new();
    let mut chat_input = String::new();
    let mut chat_input_enabled = false;

    'running: for tick in 0.. {
        // Get Health and Energy
        let (current_health, max_health) = client
            .current::<comp::Health>()
            .map_or((0, 0), |health| (health.current(), health.maximum()));
        let (current_energy, max_energy) = client
            .current::<comp::Energy>()
            .map_or((0, 0), |energy| (energy.current(), energy.maximum()));

        // Invite Logic
        let (inviter_uid, invite_kind) =
            if let Some((inviter_uid, _, _, invite_kind)) = client.invite() {
                (Some(inviter_uid), Some(invite_kind))
            } else {
                (None, None)
            };

        //Get entity username from UID
        let inviter_username = if let Some(uid) = inviter_uid {
            if let Some(entity) = client
                .state()
                .ecs()
                .read_resource::<UidAllocator>()
                .retrieve_entity_internal(uid.id())
            {
                if let Some(player) = client.state().read_storage::<comp::Player>().get(entity) {
                    player.alias.clone()
                } else {
                    "".to_string()
                }
            } else {
                "".to_string()
            }
        } else {
            "".to_string()
        };
        //Get player pos
        let player_pos = client
            .state()
            .read_storage::<comp::Pos>()
            .get(client.entity())
            .map(|pos| pos.0)
            .unwrap_or(Vec3::zero());
        let to_screen_pos = |pos: Vec2<f32>, zoom_level: f32| {
            ((pos - Vec2::from(player_pos)) * Vec2::new(1.0, -1.0) / zoom_level
                + screen_size.map(|e| e as f32) / 2.0)
                .map(|e| e as i32)
        };

        let from_screen_pos = |pos: Vec2<u16>, zoom_level: f32| {
            Vec2::from(player_pos)
                + (pos.map(|e| e as f32) - screen_size.map(|e| e as f32) / 2.0)
                    * zoom_level
                    * Vec2::new(1.0, -1.0)
        };

        let mut inputs = comp::ControllerInputs::default();

        // Handle inputs
        for c in key_rx.try_iter() {
            match c.unwrap() {
                TermEvent::Key(Key::Char(c)) if chat_input_enabled => match c {
                    '\n' => {
                        client.send_chat(chat_input.clone());
                        chat_input = String::new();
                        chat_input_enabled = false;
                    }
                    '\x08' => {
                        chat_input.pop();
                    }
                    c => chat_input.push(c),
                },
                TermEvent::Key(Key::Char('\n')) => chat_input_enabled = true,
                TermEvent::Key(Key::Char('w')) => inputs.move_dir.y += 1.0,
                TermEvent::Key(Key::Char('a')) => inputs.move_dir.x -= 1.0,
                TermEvent::Key(Key::Char('s')) => inputs.move_dir.y -= 1.0,
                TermEvent::Key(Key::Char('d')) => inputs.move_dir.x += 1.0,
                TermEvent::Key(Key::Char('u')) => client.accept_invite(),
                TermEvent::Key(Key::Char('i')) => client.decline_invite(),
                TermEvent::Mouse(me) => match me {
                    MouseEvent::Press(_, x, y) => {
                        tgt_pos = Some(from_screen_pos(Vec2::new(x, y), zoom_level))
                    }
                    _ => {}
                },
                TermEvent::Key(Key::Char(' ')) => {
                    client.handle_input(InputKind::Jump, true, None, None);
                }
                TermEvent::Key(Key::Char('x')) => {
                    client.handle_input(InputKind::Primary, true, None, None);
                }
                TermEvent::Key(Key::Char('z')) => {
                    if is_secondary_active {
                        client.handle_input(InputKind::Secondary, false, None, None);
                        is_secondary_active = false;
                    } else {
                        client.handle_input(InputKind::Secondary, true, None, None);
                        is_secondary_active = true;
                    }
                }
                TermEvent::Key(Key::Char('g')) => client.toggle_glide(), //do_glide = !do_glide,
                TermEvent::Key(Key::Char('r')) => client.respawn(),
                TermEvent::Key(Key::Char('+')) => zoom_level /= 1.5,
                TermEvent::Key(Key::Char('-')) => zoom_level *= 1.5,
                TermEvent::Key(Key::Char('q')) => break 'running,
                _ => {}
            }
        }
        if let Some(tp) = tgt_pos {
            if tp.distance_squared(player_pos.into()) < 1.0 {
                tgt_pos = None;
            } else {
                inputs.move_dir = (tp - Vec2::from(player_pos))
                    .try_normalized()
                    .unwrap_or(Vec2::zero());
            }
        }

        // Tick client
        for event in client.tick(inputs, clock.dt(), |_| ()).unwrap() {
            match event {
                Event::Chat(msg) => match msg.chat_type {
                    comp::ChatType::World(_) => chat_log.push(msg.message),
                    comp::ChatType::Group(_, _) => {
                        chat_log.push(format!("[Group] {}", msg.message))
                    }

                    _ => {}
                },
                _ => {}
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
                    let wpos = (player_pos
                        + Vec3::new(i, j, 0)
                            .map2(screen_size.into(), |e, sz: u16| e as f32 - sz as f32 / 2.0)
                            * Vec2::new(1.0, -1.0)
                            * zoom_level)
                        .map(|e| e.floor() as i32);

                    let mut block_z = 0;
                    let mut block = None;
                    let mut block_char = None;

                    for (k, z) in (-2..16).enumerate() {
                        block_z = wpos.z - z;

                        if let Some(b) = state.terrain().get(wpos + Vec3::unit_z() * -z).ok() {
                            let sprite = b.get_sprite();
                            if sprite.is_some() && sprite.unwrap() != SpriteKind::Empty {
                                let sprite2 = sprite.unwrap();
                                let flower1 =
                                    SpriteKind::BarrelCactus as u8..=SpriteKind::Turnip as u8;
                                let flower2 =
                                    SpriteKind::LargeGrass as u8..=SpriteKind::LargeCactus as u8;
                                let furniture =
                                    SpriteKind::Window1 as u8..=SpriteKind::WardrobeDouble as u8;
                                block_char = match sprite2 {
                                    SpriteKind::Apple => Some('a'),
                                    SpriteKind::Sunflower => Some('u'),
                                    SpriteKind::Mushroom => Some('m'),
                                    SpriteKind::Velorite | SpriteKind::VeloriteFrag => Some('v'),
                                    SpriteKind::Chest | SpriteKind::Crate => Some('c'),
                                    SpriteKind::Stones => Some('s'),
                                    SpriteKind::Twigs => Some('t'),
                                    SpriteKind::Amethyst | SpriteKind::Ruby => Some('g'), // TODO: add more
                                    SpriteKind::Beehive => Some('b'),
                                    _ => {
                                        let sprite3 = sprite2 as u8;
                                        if flower1.contains(&sprite3) || flower2.contains(&sprite3)
                                        {
                                            Some('%')
                                        } else if furniture.contains(&sprite3) {
                                            Some('&')
                                        } else {
                                            None
                                        }
                                    }
                                };
                            } else if b.is_filled() {
                                block = Some(*b);
                                if block_char.is_none() {
                                    block_char = Some(if k < level_chars.len() {
                                        level_chars[k as usize]
                                    } else {
                                        if block_z % 2 == 0 {
                                            'O'
                                        } else {
                                            '0'
                                        }
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

                    if block_char.is_none() {
                        block_char = Some('?');
                    }

                    write!(
                        display,
                        "{}{}",
                        color::Rgb(col.r, col.g, col.b).fg_string(),
                        block_char.unwrap()
                    )
                    .unwrap();
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
                    let character = match body.unwrap() {
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
                        Body::BipedSmall(_) => '2',
                        Body::Object(_) => 'o',
                        Body::Golem(_) => 'G',
                        Body::Dragon(_) => 'S',
                        Body::Theropod(_) => 'T',
                        Body::Ship(_) => '_',
                        //_ => '?'
                    };

                    if scr_pos
                        .map2(screen_size, |e, sz| e >= 0 && e < sz as i32)
                        .reduce_and()
                    {
                        write!(
                            display.at((scr_pos.x as u16, scr_pos.y as u16)),
                            "{}{}",
                            color::White.fg_str(),
                            character
                        )
                        .unwrap();
                    }
                }
            }

            write!(
                display.at((0, screen_size.y + 0)),
                "/------- Controls ------\\"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 1)),
                "|   wasd - Move         |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 2)),
                "|  click - Move         |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 3)),
                "|  space - Jump         |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 4)),
                "|      x - Attack1      |"
            )
            .unwrap();
            if is_secondary_active {
                write!(
                    display.at((0, screen_size.y + 5)),
                    "|  z - Attack2 ACTIVE   |"
                )
            } else {
                write!(
                    display.at((0, screen_size.y + 5)),
                    "|  z - Attack2 INACTIVE |"
                )
            }
            .unwrap();
            write!(
                display.at((0, screen_size.y + 6)),
                "|      g - toggle glide |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 7)),
                "|      r - Respawn      |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 8)),
                "|      q - Quit         |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 9)),
                "|      + - Zoom in      |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 10)),
                "|      - - Zoom out     |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 11)),
                "| return - Chat         |"
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 12)),
                "{}",
                &format!("Current Health - {}/{}", current_health, max_health)
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 13)),
                "{}",
                &format!("Current Energy - {}/{}", current_energy, max_energy)
            )
            .unwrap();
            write!(
                display.at((0, screen_size.y + 14)),
                "\\------------------------/"
            )
            .unwrap();
            if inviter_uid.is_some() {
                write!(
                    display.at((0, screen_size.y + 15)),
                    "{:?}",
                    &format!(
                        "You've been invited to {:?} by {:?}. Accept [U] or Decline [I]?",
                        invite_kind, inviter_username
                    )
                )
            } else {
                write!(display.at((0, screen_size.y + 15)), " ")
            }
            .unwrap();

            let clear = "                                                                ";
            for (i, msg) in chat_log.iter().rev().take(10).enumerate() {
                write!(display.at((24, screen_size.y + 10 - i as u16)), "{}", clear).unwrap();
                write!(
                    display.at((24, screen_size.y + 10 - i as u16)),
                    "{}",
                    msg.get(0..48).unwrap_or(&msg)
                )
                .unwrap();
            }
            write!(display.at((24, screen_size.y + 12)), "{}", clear).unwrap();
            write!(display.at((24, screen_size.y + 12)), "> {}", chat_input).unwrap();
        }

        // Finish drawing
        display.flush();

        // Wait for next tick
        clock.tick();
    }
}
