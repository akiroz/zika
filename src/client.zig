const std = @import("std");
const mqtt = @import("mqtt.zig");
const driver = @import("driver.zig");
const config = @import("config.zig");

const Allocator = std.mem.Allocator;
const Ip4Address = std.net.Ip4Address;
const Base64UrlEncoder = std.base64.url_safe_no_pad.Encoder;
const Config = config.Config;
const NetInterface = driver.NetInterface;
const IpHeader = driver.IpHeader;

pub const Tunnel = struct {
    const Self = @This();
    const Conf = config.ClientTunnel;

    conf: Conf,
    id: []u8,
    topic: []u8,
    bind_addr: u32,
    
    pub fn create(alloc: *Allocator, conf: Conf) !*Self {
        const self = try alloc.create(Self);
        errdefer alloc.destroy(self);

        self.conf = conf;
        
        const ip = try Ip4Address.parse(conf.bind_addr, 0);
        self.bind_addr = ip.sa.addr;
        
        self.id = try alloc.alloc(u8, conf.id_length);
        errdefer alloc.free(self.id);
        std.crypto.random.bytes(self.id);
        
        const b64_len = Base64UrlEncoder.calcSize(conf.id_length);
        const b64_id = try alloc.alloc(u8, b64_len);
        defer alloc.free(b64_id);
        _ = Base64UrlEncoder.encode(b64_id, self.id);
        
        self.topic = try std.fmt.allocPrint(alloc, "{s}/{s}", .{conf.topic, b64_id});
        errdefer alloc.free(self.topic);

        std.log.info("Tunnel: {s} -> {s} ({s})", .{conf.bind_addr, conf.topic, b64_id});
        return self;
    }

    pub fn handle_local(self: *Self, pkt: []u8) !bool {
        const hdr = @ptrCast(*IpHeader, pkt);
        if(hdr.dst != self.bind_addr) return false;
        // handle packet
        return true;
    }

    pub fn handle_remote(self: *Self, pkt: []u8) !*Self {
    }
};

pub const Client = struct {
    const Self = @This();
    const Error = error {
        ConfigMissing
    };
    
    ifce: NetInterface(*Self),
    tunnels: []*Tunnel,

    pub fn init(alloc: *Allocator, conf: * const Config) !*Self {
        const client_conf = conf.client orelse {
            std.log.err("missing client config", .{});
            return Error.ConfigMissing;
        };

        const self = try alloc.create(Self);
        errdefer alloc.destroy(self);

        self.tunnels = try alloc.alloc(*Tunnel, client_conf.tunnels.len);
        errdefer alloc.free(self.tunnels);

        std.log.info("== Client Config =================================", .{});
        for (client_conf.tunnels) |tunnel, idx| {
            self.tunnels[idx] = try Tunnel.create(alloc, .{
                .id_length = tunnel.id_length,
                .topic = tunnel.topic,
                .bind_addr = tunnel.bind_addr,
            });
        }
        self.ifce = try NetInterface(*Self).init(alloc, conf, self, @ptrCast(driver.PacketHandler(*Self), &recv));
        std.log.info("==================================================", .{});

        return self;
    }

    fn recv(self: *Self, pkt: []u8) void {
        for (self.tunnels) |tunnel| {
            const handled = tunnel.handle_local(pkt) catch |err| {
                std.log.warn("handle_local: {s}", .{err});
            };
            if(handled) break;
        }
    }

};

pub fn main() !void {
    const alloc = std.heap.c_allocator;
    const conf_path = try std.fs.cwd().realpathAlloc(alloc, "zika_config.json");
    const conf = try config.get(conf_path);
    const client = try Client.init(alloc, &conf);
    while (true) std.time.sleep(10_000_000_000);
}
