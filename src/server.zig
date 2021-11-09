const builtin = @import("builtin");
const std = @import("std");

const rheia = @import("rheia/lru.zig");
const mqtt = @import("mqtt.zig");
const driver = @import("driver.zig");
const config = @import("config.zig");

const Allocator = std.mem.Allocator;
const Ip4Address = std.net.Ip4Address;
const Base64UrlEncoder = std.base64.url_safe_no_pad.Encoder;
const Config = config.Config;
const Tunnel = driver.Tunnel;

pub const Server = struct {
    const Self = @This();
    const IdCache = rheia.AutoHashMap(u32, []u8, 100);
    const IpCache = rheia.AutoHashMap(u128, u32, 100);
    const Error = error {
        ConfigMissing
    };

    max_tunnels: u16,
    pool_start: Ip4Address,
    pool_end: Ip4Address,
    id_cache: IdCache,
    ip_cache: IpCache,
    tunnel: Tunnel(*Self),

    pub fn init(alloc: *Allocator, conf: * const Config) !*Self {
        const server_conf = conf.server orelse {
            std.log.err("missing server config", .{});
            return Error.ConfigMissing;
        };

        const self = try alloc.create(Self);
        errdefer alloc.destroy(self);
        
        self.max_tunnels = roundPowerOf2(server_conf.max_tunnels);
        self.pool_start = try Ip4Address.parse(server_conf.pool_start, 0);
        self.pool_end = try Ip4Address.parse(server_conf.pool_end, 0);
        self.id_cache = try IdCache.initCapacity(alloc, self.max_tunnels);
        self.ip_cache = try IpCache.initCapacity(alloc, self.max_tunnels);

        std.log.info("== Server Config =================================", .{});
        std.log.info("ID Length: {d}", .{server_conf.id_length});
        std.log.info("Topic: {s}", .{server_conf.topic});
        std.log.info("Max Tunnels: {d} (rounded to 2^n)", .{self.max_tunnels});
        std.log.info("IP Pool: {s} - {s}", .{server_conf.pool_start, server_conf.pool_end});
        self.tunnel = try Tunnel(*Self).init(alloc, conf, self, @ptrCast(driver.PacketHandler(*Self), &recv));
        std.log.info("==================================================", .{});

        return self;
    }

    fn recv(self: *Self, pkt: []u8) void {
        std.log.info("got pkt {d}", .{pkt.len});
    }

    fn allocIp(self: *Self, id: u128) u32 {
        const id_str = std.mem.zeroes([B64_ID_SIZE]u8);
        Base64UrlEncoder.encode(id_str, *@ptrCast([]u8, &id));
        // TODO
    }

    fn roundPowerOf2(val: u16) u16 {
        var v = val;
        v -= 1;
        v |= v >> 1;
        v |= v >> 2;
        v |= v >> 4;
        v |= v >> 8;
        v += 1;
        return v;
    }

};

pub fn main() !void {
    const alloc = std.heap.c_allocator;
    const conf_path = try std.fs.cwd().realpathAlloc(alloc, "zika_config.json");
    const conf = try config.get(conf_path);
    const server = try Server.init(alloc, &conf);
    while (true) std.time.sleep(10_000_000_000);
}
