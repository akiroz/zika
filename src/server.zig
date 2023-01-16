const builtin = @import("builtin");
const std = @import("std");

const mqtt = @import("mqtt.zig");
const driver = @import("driver.zig");
const config = @import("config.zig");

const Allocator = std.mem.Allocator;
const ArenaAllocator = std.heap.ArenaAllocator;
const Ip4Address = std.net.Ip4Address;
const Base64UrlEncoder = std.base64.url_safe_no_pad.Encoder;
const Config = config.Config;
const NetInterface = driver.NetInterface;
const IpHeader = driver.IpHeader;
const Mqtt = mqtt.Mqtt;

pub const Server = struct {
    const Self = @This();
    const IpCache = std.AutoHashMap(u128, u32);
    const IdCache = std.AutoHashMap(u32, u128);
    const TopicCache = std.AutoHashMap(u32, [:0]u8);
    const Error = error{ConfigMissing};

    arena: ArenaAllocator,
    alloc: Allocator,
    ifce: ?*NetInterface(*Self),
    mqtt: ?*Mqtt(*Self),

    pool_start: Ip4Address,
    pool_end: Ip4Address,
    pool_next_alloc: u32,

    id_len: u8,
    ip_cache: IpCache,
    id_cache: IdCache,

    b64_len: usize,
    topic_cstr: [:0]u8,
    topic_cache: TopicCache,

    pub fn init(parent_alloc: Allocator, conf: *const Config) !*Self {
        const server_conf = conf.server orelse {
            std.log.err("missing server config", .{});
            return Error.ConfigMissing;
        };

        const self = try parent_alloc.create(Self);
        self.arena = ArenaAllocator.init(parent_alloc);
        self.alloc = self.arena.allocator();
        self.ifce = null;
        self.mqtt = null;
        errdefer self.deinit();

        self.pool_start = try Ip4Address.parse(server_conf.pool_start, 0);
        self.pool_end = try Ip4Address.parse(server_conf.pool_end, 0);
        self.pool_next_alloc = self.pool_start.sa.addr;
        self.id_len = server_conf.id_length;
        self.ip_cache = IpCache.init(self.alloc);
        self.id_cache = IdCache.init(self.alloc);
        self.b64_len = Base64UrlEncoder.calcSize(self.id_len);
        self.topic_cstr = try std.cstr.addNullByte(self.alloc, server_conf.topic);
        self.topic_cache = TopicCache.init(self.alloc);

        std.log.info("== Server Config =================================", .{});
        std.log.info("ID Length: {d}", .{server_conf.id_length});
        std.log.info("Topic: {s}", .{server_conf.topic});
        std.log.info("IP Pool: {s} - {s}", .{ server_conf.pool_start, server_conf.pool_end });
        self.ifce = try NetInterface(*Self).init(self.alloc, conf, self, @ptrCast(driver.PacketHandler(*Self), &up));
        self.mqtt = try Mqtt(*Self).init(self.alloc, conf, self, @ptrCast(mqtt.PacketHandler(*Self), &down), 1);
        try self.mqtt.?.subscribe(self.topic_cstr, true);
        std.log.info("==================================================", .{});

        return self;
    }

    pub fn deinit(self: *Self) void {
        if (self.mqtt) |m| m.deinit();
        if (self.ifce) |i| i.deinit();
        self.arena.deinit();
    }

    pub fn run(self: *Self) !void {
        try self.mqtt.?.connect();
        try self.ifce.?.run();
    }

    fn allocIp(self: *Self, id: u128) !u32 {
        const next_addr = self.pool_next_alloc;

        const pool_start = std.mem.bigToNative(u32, self.pool_start.sa.addr);
        const pool_end = std.mem.bigToNative(u32, self.pool_end.sa.addr);
        var next_alloc = std.mem.bigToNative(u32, self.pool_next_alloc) + 1;
        if(next_alloc > pool_end) next_alloc = pool_start;
        self.pool_next_alloc = std.mem.nativeToBig(u32, next_alloc);

        if (self.id_cache.fetchRemove(next_addr)) |entry| {
            self.alloc.free(self.topic_cache.fetchRemove(next_addr).?.value);
            _ = self.ip_cache.remove(entry.value);
        }

        try self.ip_cache.put(id, next_addr);
        try self.id_cache.put(next_addr, id);

        var b64_id = try self.alloc.alloc(u8, self.b64_len);
        defer self.alloc.free(b64_id);
        var id_bytes = std.mem.toBytes(id)[0..self.id_len];
        _ = Base64UrlEncoder.encode(b64_id, id_bytes);
        const topic = try std.fmt.allocPrintZ(self.alloc, "{s}/{s}", .{ self.topic_cstr, b64_id });
        try self.topic_cache.put(next_addr, topic);

        std.log.info("alloc({s}): {X}", .{b64_id, next_addr});
        return next_addr;
    }

    fn up(self: *Self, pkt: []u8) void {
        const hdr = @ptrCast(*IpHeader, pkt);
        if (self.topic_cache.get(hdr.dst)) |topic| {
            self.mqtt.?.send(topic, pkt) catch |err| {
                std.log.warn("up: {s}", .{err});
            };
        }
    }

    fn down(self: *Self, topic: []const u8, msg: []u8) void {
        _ = topic;
        var id: u128 = 0;
        std.mem.copy(u8, @ptrCast(*[@sizeOf(u128)]u8, &id), msg[0..self.id_len]);
        const addr = self.ip_cache.get(id) orelse self.allocIp(id) catch |err| {
            std.log.err("allocIp: {any}", .{err});
            @panic("allocIp failed");
        };
        self.ifce.?.inject(addr, msg[self.id_len..]) catch |err| {
            std.log.warn("down: {s}", .{err});
        };
    }
};

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    var alloc = gpa.allocator();
    const conf_path = try std.fs.cwd().realpathAlloc(alloc, "zika_config.json");
    const conf = try config.get(alloc, conf_path);
    const server = try Server.init(alloc, &conf);
    defer server.deinit();
    defer alloc.destroy(server);
    try server.run();
}
