const std = @import("std");
const mqtt = @import("mqtt.zig");
const driver = @import("driver.zig");
const config = @import("config.zig");

const Allocator = std.mem.Allocator;
const Ip4Address = std.net.Ip4Address;
const Config = config.Config;
const Tunnel = driver.Tunnel;

pub const Client = struct {
    const Self = @This();
    const Error = error {
        ConfigMissing
    };
    
    bind_addr: u32,
    tunnel: Tunnel(*Self),

    pub fn init(alloc: *Allocator, conf: * const Config) !*Self {
        const client_conf = conf.client orelse {
            std.log.err("missing client config", .{});
            return Error.ConfigMissing;
        };

        const self = try alloc.create(Self);
        errdefer alloc.destroy(self);

        std.log.info("== Tunnel Config =================================", .{});
        std.log.info("ID Length: {d}", .{client_conf.id_length});
        for (client_conf.tunnels) |t| {
            const ip = try Ip4Address.parse(t.bind_addr, 0);
            self.bind_addr = ip.sa.addr;
            std.log.info("Bind: {s} to {s}", .{t.bind_addr, t.topic});
        }
        self.tunnel = try Tunnel(*Self).init(alloc, conf, self, @ptrCast(driver.PacketHandler(*Self), &recv));
        std.log.info("==================================================", .{});

        return self;
    }

    fn recv(self: *Self, pkt: []u8) void {
        std.log.info("got pkt {d}", .{pkt.len});
        self.tunnel.write(self.bind_addr, pkt) catch |err| {
            std.log.info("write failed: {s}", .{err});
        };
    }

};

pub fn main() !void {
    const alloc = std.heap.c_allocator;
    const conf_path = try std.fs.cwd().realpathAlloc(alloc, "zika_config.json");
    const conf = try config.get(conf_path);
    const client = try Client.init(alloc, &conf);
    while (true) std.time.sleep(10_000_000_000);
}
