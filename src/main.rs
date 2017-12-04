#![feature(conservative_impl_trait)]

extern crate dotenv;
extern crate telebot;
extern crate tokio_core;
extern crate futures;
extern crate tokio_timer;

use telebot::bot::RcBot;
use telebot::functions::FunctionMessage;
use tokio_core::reactor::Core;
use futures::{Future, Sink, Stream};
use futures::future::lazy;
use futures::sync::mpsc::{Receiver, Sender, channel};
use tokio_timer::Timer;
use dotenv::dotenv;
use std::env;
use std::time::{Duration, Instant};
use std::collections::HashMap;

struct State {
    bot: RcBot,
    sender: Sender<GameMessage>,
    receiver: Receiver<GameMessage>,
}

impl State {
    fn new(bot: RcBot, sender: Sender<GameMessage>, receiver: Receiver<GameMessage>) -> Self {
        State {
            bot,
            sender,
            receiver,
        }
    }
}

#[derive(PartialEq, Debug, Clone)]
enum GameState {
    Start,
    Hold(Instant),
    Win(Instant),
    Pong(Instant, u64, String),
}

impl GameState {
    fn new() -> Self {
        GameState::Start
    }

    fn hold() -> Self {
        GameState::Hold(Instant::now())
    }

    fn win() -> Self {
        GameState::Win(Instant::now())
    }

    fn ping(self, username: String) -> Option<Self> {
        match self {
            GameState::Start => Some(GameState::Pong(Instant::now(), 0, username)),
            GameState::Hold(end) => {
                let current_time = Instant::now();

                if current_time - end < Duration::from_secs(5) {
                    // Wait 5 seconds between games
                    None
                } else {
                    Some(GameState::Pong(current_time, 0, username))
                }
            }
            GameState::Win(end) => {
                let current_time = Instant::now();

                if current_time - end < Duration::from_secs(5) {
                    // Wait 5 seconds between games
                    Some(GameState::Win(end))
                } else {
                    Some(GameState::Pong(current_time, 0, username))
                }
            }
            GameState::Pong(_, counter, prev_username) => {
                if username == prev_username {
                    None
                } else {
                    Some(GameState::Pong(Instant::now(), counter + 1, username))
                }
            }
        }
    }

    fn counter(&self) -> u64 {
        match *self {
            GameState::Pong(_, counter, _) => counter,
            _ => 0,
        }
    }
}

enum GameMessage {
    Ping(i64, String),
    TimesUp(i64, u64),
    RemoveHold(i64),
}

impl GameMessage {
    fn ping(chat_id: i64, username: String) -> Self {
        GameMessage::Ping(chat_id, username)
    }

    fn times_up(chat_id: i64, counter: u64) -> Self {
        GameMessage::TimesUp(chat_id, counter)
    }

    fn remove_hold(chat_id: i64) -> Self {
        GameMessage::RemoveHold(chat_id)
    }
}

fn winner(bot: &RcBot, chat_id: i64, user: &str) {
    bot.inner.handle.spawn(
        bot.message(chat_id, format!("{} wins", user))
            .send()
            .map(|_| ())
            .map_err(|e| println!("Error: {:?}", e)),
    )
}

fn user_pinged(
    bot: &RcBot,
    game_map: &mut HashMap<i64, GameState>,
    chat_id: i64,
    user: String,
    tx: Sender<GameMessage>,
) {
    let state = match game_map.get(&chat_id) {
        Some(game @ &GameState::Pong(..)) => game.clone().ping(user),
        Some(game) => {
            let game = game.clone().ping(user);
            if let Some(GameState::Pong(..)) = game {
                bot.inner.handle.spawn(
                    bot.message(chat_id, "New game!".into())
                        .send()
                        .map(|_| ())
                        .map_err(|e| println!("Error: {:?}", e)),
                );
            }
            game
        }
        None => {
            bot.inner.handle.spawn(
                bot.message(chat_id, "New game!".into())
                    .send()
                    .map(|_| ())
                    .map_err(|e| println!("Error: {:?}", e)),
            );
            GameState::new().ping(user)
        }
    };

    let timer = Timer::default();

    if let Some(game @ GameState::Pong(..)) = state {
        let counter = game.counter();

        bot.inner.handle.spawn(
            timer
                .sleep(Duration::from_secs(3))
                .map_err(|e| println!("Error: {}", e))
                .and_then(move |_| {
                    tx.send(GameMessage::times_up(chat_id, counter))
                        .map(|_| ())
                        .map_err(|e| println!("Error: {}", e))
                }),
        );
        bot.inner.handle.spawn(
            bot.message(chat_id, "pong".into())
                .send()
                .map(|_| ())
                .map_err(|e| println!("Error: {:?}", e)),
        );
        game_map.insert(chat_id, game);
    } else if let Some(GameState::Win(..)) = state {
        bot.inner.handle.spawn(
            bot.message(chat_id, "Please wait 5 seconds".into())
                .send()
                .map(|_| ())
                .map_err(|e| println!("Error: {:?}", e)),
        );
    } else {
        bot.inner.handle.spawn(
            timer
                .sleep(Duration::from_secs(5))
                .map_err(|e| println!("Error: {}", e))
                .and_then(move |_| {
                    tx.send(GameMessage::remove_hold(chat_id))
                        .map(|_| ())
                        .map_err(|e| println!("Error: {}", e))
                }),
        );
        bot.inner.handle.spawn(
            bot.message(chat_id, "Don't ping your own pong. You lose.".into())
                .send()
                .map(|_| ())
                .map_err(|e| println!("Error: {:?}", e)),
        );
        game_map.insert(chat_id, GameState::hold());
    }
}

fn times_up(
    bot: &RcBot,
    game_map: &mut HashMap<i64, GameState>,
    chat_id: i64,
    timeout_counter: u64,
    tx: Sender<GameMessage>,
) {
    let game = if let Some(game) = game_map.get(&chat_id) {
        Some(game.clone())
    } else {
        None
    };

    if let Some(GameState::Pong(_, counter, username)) = game {
        if counter == timeout_counter {
            bot.inner.handle.spawn(
                Timer::default()
                    .sleep(Duration::from_secs(5))
                    .map_err(|e| println!("Error: {}", e))
                    .and_then(move |_| {
                        tx.send(GameMessage::remove_hold(chat_id))
                            .map(|_| ())
                            .map_err(|e| println!("Error: {}", e))
                    }),
            );
            winner(bot, chat_id, &username);
            game_map.insert(chat_id, GameState::win());
        }
    }
}

fn remove_hold(game_map: &mut HashMap<i64, GameState>, chat_id: i64) {
    match game_map.get(&chat_id) {
        Some(&GameState::Hold(..)) |
        Some(&GameState::Win(..)) => {
            game_map.remove(&chat_id);
        }
        _ => (),
    }
}

fn game_thread(init: State) -> impl Future<Item = (), Error = ()> {
    let State {
        bot,
        sender,
        receiver,
    } = init;

    receiver
        .fold(HashMap::new(), move |mut game, message| match message {
            GameMessage::Ping(chat_id, user) => {
                user_pinged(&bot.clone(), &mut game, chat_id, user, sender.clone());

                Ok(game)
            }
            GameMessage::TimesUp(chat_id, timeout_counter) => {
                times_up(
                    &bot.clone(),
                    &mut game,
                    chat_id,
                    timeout_counter,
                    sender.clone(),
                );

                Ok(game)
            }
            GameMessage::RemoveHold(chat_id) => {
                remove_hold(&mut game, chat_id);

                Ok(game)
            }
        } as
            Result<HashMap<_, _>, ()>)
        .map(|_| ())
}

fn main() {
    dotenv().ok();

    let token = env::var("TELEGRAM_BOT_TOKEN").expect(
        "Please set the TELEGRAM_BOT_TOKEN environment variable",
    );
    let mut lp = Core::new().unwrap();
    let bot = RcBot::new(lp.handle(), &token).update_interval(50);

    let (tx, rx) = channel(100);
    let tx_clone = tx.clone();

    lp.handle().spawn(game_thread(
        State::new(bot.clone(), tx_clone, rx),
    ));

    let handle = bot.new_cmd("/ping")
        .map_err(|e| println!("Failed to get command: {:?}", e))
        .and_then(move |(_bot, msg)| {
            let tx = tx.clone();

            let chat_id = msg.chat.id;

            lazy(move || if let Some(user) = msg.from {
                if let Some(username) = user.username {
                    Ok((chat_id, format!("@{}", username)))
                } else {
                    Ok((chat_id, user.first_name))
                }
            } else {
                Err(())
            }).and_then(move |(chat_id, name)| {
                tx.clone()
                    .send(GameMessage::ping(chat_id, name))
                    .map(|_| ())
                    .map_err(|e| println!("Failed to send GameMessage: {}", e))
            })
        });

    bot.register(handle);

    lp.run(
        bot.get_stream()
            .map(|_| ())
            .or_else(|e| {
                println!("Error: {:?}", e);
                Ok(()) as Result<(), ()>
            })
            .for_each(|_| Ok(())),
    ).unwrap();
}
