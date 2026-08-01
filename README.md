# Terralistic

Discord server: https://discord.gg/WANEYJAxAB

To build and run the client:

```bash
cargo run
```

To build and run in release mode:

```bash
cargo run --release
```


To run the server use the above commands and append `-- server`

<img width="1782" alt="Screenshot 2023-11-25 at 5 10 02 PM" src="https://github.com/Zorz42/Terralistic/assets/54270248/7b723998-fd24-4daf-9037-fcb8032b6738">

## Known Bugs

- **Player rubber-banding** - When moving quickly (jumping, falling into caves) the player can snap back to a previous position due to client-server position desync.
- **Saving while falling** - The player state can be saved mid-fall, causing them to respawn/reload in mid-air or at an unexpected position.

## Features Roadmap

- **Step up single blocks** - Allow the player to automatically step up single-block-high ledges instead of requiring a full jump.
- **Block breaking QoL** - Quality-of-life improvements to block breaking, such as hold-to-break and wall breaking.

