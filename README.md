# Zika

IP Tunneling over MQTT

Supports 2 network drivers:
- Linux: TUN driver
- MacOS: BPF driver (via libpcap)

Server: can handle multiple tunnels from multiple clients, each mapped to a different local IP

Client: can create multiple tunnels to multiple servers, each mapped to a different local IP

