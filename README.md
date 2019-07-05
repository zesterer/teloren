# Teloren ('Telnet' + 'Veloren')

Teloren is an ANSI-compatible terminal frontend to Veloren, the multiplayer RPG voxel game written in Rust.
Teloren is currently still very much a work in progress and I strongly recommend you play with the [3D frontend](https://www.veloren.net) first.

![alt text](misc/screenshot.png "Teloren")

## Usage

Start Teloren using the following arguments:

```
teloren --alias YOUR_ALIAS
```

Optionally, you may also specify `--server` and `--port` arguments to play on something other than the main public server.

## Status

Currently implemented

- World rendering
- Basic movement

To be implemented

- Chat
- Build mode
- Lighting
- Alias overlays, HUD

## Why?

Veloren's engine has been deliberately designed in an extremely modular manner.
As a result, graphical frontends are entirely decoupled from the client library itself, and so writing alternative frontends for the game is actually quite easy.
