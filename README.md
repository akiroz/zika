[![Crates.io](https://img.shields.io/crates/v/zika.svg)](https://crates.io/crates/zika)
[![docs.rs](https://docs.rs/zika/badge.svg)](https://docs.rs/zika)
[![Build Status Github](https://github.com/akiroz/zika/actions/workflows/ci.yaml/badge.svg?branch=master)](https://github.com/akiroz/zika/actions/workflows/ci.yaml)

# Zika

IP Tunneling over MQTT

Designed to allow remote access for IoT/Edge devices connected to an existing MQTT network.

### Working Mechanism

IP packets are sent as MQTT payloads on 2 topics:
- Client -> Server: `<topic>` (payload prepended by tunnel ID)
- Server -> Client: `<topic>/<base64-tunnel-id>`

Server: can handle multiple tunnels from multiple clients, each mapped to a different local IP

Client: can create multiple tunnels to multiple servers, each mapped to a different local IP

The MQTT connection is assumed to be secure & have authentication mechanisms in place.
Zika offers no extra layers of security on top of the MQTT connection,
it's possible capture/inject arbitrary IP packets to/from the target device if the MQTT connection/broker is compromised.

### Config & Run

See [zika_config.example.toml](zika_config.example.toml)

Copy `zika-client` and `zika_config.toml` to the same directory

Run `zika-client`

- Linux: `setcap cap_net_admin+eip zika-client`
- macOS: requires `sudo`
- Windows: run as Administrator, requires `wintun.dll` in same directory

### Building

```
$ cargo build
```
