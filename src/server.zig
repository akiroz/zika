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
    const Error = error {
        ConfigMissing
    };

    arena: ArenaAllocator,
    ifce: ?*NetInterface(*Self),
    mqtt: ?*Mqtt(*Self),
    
    pool_start: Ip4Address,
    pool_end: Ip4Address,
    pool_next_alloc: u32,
    
    id_len: u8,
    ip_cache: IpCache,
    id_cache: IdCache,
    
    b64_len: usize,
    topic: []const u8,
    topic_cache: TopicCache,

    pub fn init(parent_alloc: *Allocator, conf: *const Config) !*Self {
        const server_conf = conf.server orelse {
            std.log.err("missing server config", .{});
            return Error.ConfigMissing;
        };

        var arena = ArenaAllocator.init(parent_alloc);
        const self = try arena.allocator.create(Self);
        self.arena = arena;
        self.ifce = null;
        self.mqtt = null;
        errdefer self.deinit();
        const alloc = &self.arena.allocator;
        
        self.pool_start = try Ip4Address.parse(server_conf.pool_start, 0);
        self.pool_end = try Ip4Address.parse(server_conf.pool_end, 0);
        self.pool_next_alloc = self.pool_start.sa.addr;
        self.id_len = server_conf.id_length;
        self.ip_cache = IpCache.init(alloc);
        self.id_cache = IdCache.init(alloc);
        self.b64_len = Base64UrlEncoder.calcSize(self.id_len);
        self.topic = server_conf.topic;
        self.topic_cache = TopicCache.init(alloc);

        std.log.info("== Server Config =================================", .{});
        std.log.info("ID Length: {d}", .{server_conf.id_length});
        std.log.info("Topic: {s}", .{server_conf.topic});
        std.log.info("IP Pool: {s} - {s}", .{server_conf.pool_start, server_conf.pool_end});
        self.ifce = try NetInterface(*Self).init(alloc, conf, self, @ptrCast(driver.PacketHandler(*Self), &up));
        self.mqtt = try Mqtt(*Self).init(alloc, conf, self, @ptrCast(mqtt.PacketHandler(*Self), &down), 1);
        std.log.info("==================================================", .{});

        return self;
    }

    pub fn deinit(self: *Self) void {
        if(self.mqtt) |m| m.deinit();
        if(self.ifce) |i| i.deinit();
        self.arena.deinit();
    }

    pub fn run(self: *Self) !void {
        try self.mqtt.?.connect();
        try self.ifce.?.run();
    }

    fn allocIp(self: *Self, id: u128) !u32 {
        const alloc = &self.arena.allocator;
        const next_addr = self.pool_next_alloc;
        self.pool_next_alloc += 1;
        if(self.pool_next_alloc > self.pool_end.sa.addr) {
            self.pool_next_alloc = self.pool_start.sa.addr;
        }

        if(self.id_cache.fetchRemove(next_addr)) |entry| {
            alloc.free(self.b64_cache.fetchRemove(next_addr).?.value);
            _ = self.ip_cache.remove(entry.value);
        }

        try self.ip_cache.put(id, next_addr);
        try self.id_cache.put(next_addr, id);
        
        var b64_id = try alloc.alloc(u8, self.b64_len);
        defer alloc.free(b64_id);
        var id_bytes = std.mem.toBytes(id);
        id_bytes.len = self.id_len;
        Base64UrlEncoder.encode(b64_id, id_bytes);
        const topic = try std.fmt.allocPrint(alloc, "{s}/{s}", .{self.topic, b64_id});
        try self.topic_cache.put(next_addr, topic);

        return next_addr;
    }

    fn up(self: *Self, pkt: []u8) void {
        const hdr = @ptrCast(*IpHeader, pkt);
        if(self.topic_cache.get(hdr.dst)) |topic| {
            self.mqtt.?.send(topic, pkt) catch |err| {
                std.log.warn("up: {s}", .{err});
            };
        }
    }

    fn down(self: *Self, topic: []const u8, msg: []u8) void {
        var id: u128 = 0;
        std.mem.copy(u8, std.mem.toBytes(id)[0..], msg[0..self.id_len]);
        if(self.ip_cache.get(id)) |addr| {
            self.ifce.?.inject(addr, msg[self.id_len..]) catch |err| {
                std.log.warn("down: {s}", .{err});
            };
        }
    }

};

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    const alloc = &gpa.allocator;
    const conf_path = try std.fs.cwd().realpathAlloc(alloc, "zika_config.json");
    const conf = try config.get(alloc, conf_path);
    const server = try Server.init(alloc, &conf);
    try server.run();
}
