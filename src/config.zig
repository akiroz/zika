const std = @import("std");

const MqttOptions = struct {
    connect_timeout: u32 = 30,
    keep_alive_interval: u32 = 60,
    reconnect_interval_min: u32 = 1,
    reconnect_interval_max: u32 = 60,

    username: ?[]const u8 = null,
    password: ?[]const u8 = null,

    ca: ?[]const u8 = null,
    auth_server: bool = true,
    key: ?[]const u8 = null,
    cert: ?[]const u8 = null,
};

const MqttBroker = struct {
    options: ?MqttOptions = null,
    url: []const u8,
};

const MqttConfig = struct {
    options: MqttOptions = .{},
    brokers: []MqttBroker,
};

const DriverPcap = struct {
    interface: []const u8,
};

const DriverTun = struct {
    netmask: []const u8,
};

const DriverConfig = struct {
    local_addr: []const u8,
    pcap: ?DriverPcap = null,
    tun: ?DriverTun = null,
};

const ServerConfig = struct {
    id_length: u8 = 4,
    topic: []const u8,
    max_tunnels: u16 = 16,
    pool_start: []const u8,
    pool_end: []const u8,
};

const ClientTunnel = struct {
    topic: []const u8,
    bind_addr: []const u8,
};

const ClientConfig = struct {
    id_length: u8 = 4,
    tunnels: []ClientTunnel,
};

pub const Config = struct {
    mqtt: MqttConfig,
    driver: DriverConfig,
    server: ?ServerConfig = null,
    client: ?ClientConfig = null,
};

pub fn get(file: []u8) !Config {
    const cfg_file = try std.fs.openFileAbsolute(file, .{ .read = true });
    defer cfg_file.close();

    const file_size = try cfg_file.getEndPos();
    const cfg_buf = try std.heap.c_allocator.alloc(u8, file_size);
    defer std.heap.c_allocator.free(cfg_buf);

    _ = try cfg_file.readAll(cfg_buf);
    var stream = std.json.TokenStream.init(cfg_buf);

    @setEvalBranchQuota(2000);
    return try std.json.parse(Config, &stream, .{
        .allocator = std.heap.c_allocator,
        .duplicate_field_behavior = .UseLast,
        .ignore_unknown_fields = true,
        .allow_trailing_data = true,
    });
}