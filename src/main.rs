#![feature(conservative_impl_trait)]

extern crate dotenv;
extern crate telebot;
extern crate tokio_core;
extern crate futures;

use telebot::bot::RcBot;
use telebot::functions::FunctionMessage;
use tokio_core::reactor::Core;
use futures::{Future, Sink, Stream};
use futures::future::lazy;
use futures::sync::mpsc::{Receiver, channel};
use dotenv::dotenv;
use std::env;
use std::time::{Duration, Instant};
use std::thread;
use std::collections::HashMap;

struct State {
    bot: RcBot,
    receiver: Receiver<GameMessage>,
}

impl State {
    fn new(bot: RcBot, receiver: Receiver<GameMessage>) -> Self {
        State {
            bot: bot,
            receiver: receiver,
        }
    }
}

#[derive(PartialEq, Debug, Clone)]
enum GameState {
    None,
    Pong(String),
}

impl GameState {
    fn new() -> Self {
        GameState::None
    }

    fn ping(self, username: String) -> Self {
        GameState::Pong(username)
    }
}

enum GameMessage {
    Ping(i64, String),
    Check,
}

impl GameMessage {
    fn ping(chat_id: i64, username: String) -> Self {
        GameMessage::Ping(chat_id, username)
    }

    fn check() -> Self {
        GameMessage::Check
    }
}

fn winner(bot: RcBot, chat_id: i64, user: &str) {
    bot.inner.handle.spawn(
        bot.message(chat_id, format!("{} wins", user))
            .send()
            .map(|_| ())
            .map_err(|e| println!("Error: {:?}", e)),
    )
}

fn user_pinged(
    bot: RcBot,
    game_map: &mut HashMap<i64, (Instant, GameState)>,
    chat_id: i64,
    user: String,
) {
    let state = match game_map.get(&chat_id) {
        Some(game_tup) => {
            let state = game_tup.clone();
            (Instant::now(), state.1.ping(user))
        }
        None => {
            bot.inner.handle.spawn(
                bot.message(chat_id, "New game!".into())
                    .send()
                    .map(|_| ())
                    .map_err(|e| println!("Error: {:?}", e)),
            );
            (Instant::now(), GameState::new().ping(user))
        }
    };

    game_map.insert(chat_id, state);
}

fn check(
    bot: RcBot,
    duration: Duration,
    chat_id: i64,
    previous_time: Instant,
    game: &GameState,
) -> bool {
    let current_time = Instant::now();
    if current_time - previous_time > duration {
        if let &GameState::Pong(ref user) = game {
            winner(bot, chat_id, user);
        }
        false
    } else {
        true
    }
}

fn game_thread(init: State) -> impl Future<Item = (), Error = ()> {
    let State { bot, receiver } = init;

    let duration = Duration::from_secs(3);

    receiver
        .fold(HashMap::new(), move |mut game, message| match message {
            GameMessage::Ping(chat_id, user) => {
                user_pinged(bot.clone(), &mut game, chat_id, user);

                Ok(game)
            }
            GameMessage::Check => {
                game.retain(|k, &mut (v0, ref mut v1)| {
                    check(bot.clone(), duration.clone(), k.clone(), v0, &v1)
                });
                Ok(game)
            }
        })
        .map(|_| ())
        .map_err(|_| ())
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

    let pong_thread = thread::spawn(move || {
        let mut lp = Core::new().unwrap();
        let bot = RcBot::new(lp.handle(), &token);
        lp.run(game_thread(State::new(bot.clone(), rx))).unwrap();
    });

    let handle = bot.new_cmd("/ping")
        .map_err(|e| println!("Failed to get command: {:?}", e))
        .and_then(move |(bot, msg)| {
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
                    .map(move |_| (bot.clone(), chat_id))
                    .map_err(|e| println!("Failed to send GameMessage: {}", e))
            })
        })
        .and_then(|(bot, chat_id)| {
            bot.message(chat_id, "pong".into())
                .send()
                .map(|_| ())
                .map_err(|e| println!("Error: {:?}", e))
        });

    bot.register(handle);

    let timer_thread = thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(1000));
        let res = tx_clone.clone().send(GameMessage::check()).wait();

        match res {
            Ok(_) => (),
            Err(e) => println!("Error: {}", e),
        }
    });

    bot.run(&mut lp).unwrap();

    let _ = timer_thread.join();
    let _ = pong_thread.join();
}
