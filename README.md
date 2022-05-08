# Zika

![](https://img.shields.io/badge/zig-0.9.1-informational)

IP Tunneling over MQTT

Designed to allow remote access for IoT/Edge devices connected to an existing MQTT network.

IP packets are sent as MQTT payloads on 2 topics:
- Client -> Server: `<topic>` (payload prepended by tunnel ID)
- Server -> Client: `<topic>/<base64-tunnel-id>`

Server: can handle multiple tunnels from multiple clients, each mapped to a different local IP

Client: can create multiple tunnels to multiple servers, each mapped to a different local IP

The MQTT connection is assumed to be secure & have authentication mechanisms in place.
Zika offers no extra layers of security on top of the MQTT connection,
it's possible capture/inject arbitrary IP packets to/from the target device if the MQTT connection/broker is compromised.

Currently supports 2 network drivers:
- Linux: TUN driver
- MacOS: BPF driver (via libpcap)

### Configuration

See [zika_config.sample.json](zika_config.sample.json)

### Building

Dependencies:
- libmosquitto
- libpcap (MacOS only, built-in)
- linux-headers (Linux only)

```
$ zig build
```
