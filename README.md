# Telegram PingPong

Play ping-pong with your friends!

Example:
```
you: /ping
bot: pong
friend: /ping
bot: pong
bot: @friend wins!


friend: /ping
bot: pong
you: /ping
bot: pong
friend: /ping
bot: pong
...
bot: [somebody] wins!
```

### How it works
When a game is started with `/ping`, someone else in the chat has 3 seconds to
respond with /ping, or the first person wins.
