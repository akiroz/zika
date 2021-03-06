const std = @import("std");
const Allocator = std.mem.Allocator;

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
    brokers: []MqttBroker,
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
    tunnels: []ClientTunnel,
};

pub const Config = struct {
    mqtt: MqttConfig,
    driver: DriverConfig,
    server: ?ServerConfig = null,
    client: ?ClientConfig = null,
};

pub fn get(alloc: Allocator, file: []u8) !Config {
    const cfg_file = try std.fs.openFileAbsolute(file, .{ .read = true });
    defer cfg_file.close();

    const file_size = try cfg_file.getEndPos();
    const cfg_buf = try alloc.alloc(u8, file_size);
    defer alloc.free(cfg_buf);

    _ = try cfg_file.readAll(cfg_buf);
    var stream = std.json.TokenStream.init(cfg_buf);

    @setEvalBranchQuota(2000);
    return try std.json.parse(Config, &stream, .{
        .allocator = alloc,
        .duplicate_field_behavior = .UseLast,
        .ignore_unknown_fields = true,
        .allow_trailing_data = true,
    });
}