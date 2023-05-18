const std = @import("std");
const Allocator = std.mem.Allocator;

const Error = error{
    MissingConfig,
};

pub const MqttOptions = struct {
    keepalive_interval: u16 = 60,
    reconnect_interval_min: u16 = 1,
    reconnect_interval_max: u16 = 60,

    username: ?[]const u8 = null,
    password: ?[]const u8 = null,

    ca_file: ?[]const u8 = null,
    tls_insecure: bool = false,
    key_file: ?[]const u8 = null,
    cert_file: ?[]const u8 = null,
};

pub const MqttBroker = struct {
    options: ?MqttOptions = null,
    host: []const u8,
    port: u16,
};

pub const MqttConfig = struct {
    options: MqttOptions = .{},
    brokers: []const MqttBroker,
};

pub const DriverPcap = struct {
    interface: []const u8,
};

pub const DriverTun = struct {
    netmask: []const u8,
};

pub const DriverConfig = struct {
    local_addr: []const u8,
    pcap: ?DriverPcap = null,
    tun: ?DriverTun = null,
};

pub const ServerConfig = struct {
    id_length: u8 = 4,
    topic: []const u8,
    pool_start: []const u8,
    pool_end: []const u8,
};

pub const ClientTunnel = struct {
    id_length: u8 = 4,
    topic: []const u8,
    bind_addr: []const u8,
};

pub const ClientConfig = struct {
    tunnels: []const ClientTunnel,
};

pub const Config = struct {
    mqtt: MqttConfig,
    driver: DriverConfig,
    server: ?ServerConfig = null,
    client: ?ClientConfig = null,
};

pub fn get(alloc: Allocator, file: []u8) !Config {
    const cfg_file = std.fs.cwd().readFileAlloc(alloc, file, 4096) catch |err| {
        std.log.err("Failed to open {s}: {any}", .{ file, err });
        return Error.MissingConfig;
    };
    var json_stream = std.json.TokenStream.init(cfg_file);

    @setEvalBranchQuota(2000);
    return try std.json.parse(Config, &json_stream, .{
        .allocator = alloc,
        .duplicate_field_behavior = .UseLast,
        .ignore_unknown_fields = true,
        .allow_trailing_data = true,
    });
}
