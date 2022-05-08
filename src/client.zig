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
const ConnectHandler = mqtt.ConnectHandler;

pub fn MessageHook(comptime T: type) type {
    // user, message -> detach
    return *const fn (T, []const u8, []const u8) bool;
}

pub fn Tunnel(comptime T: type) type {
    return struct {
        const Self = @This();
        const Conf = config.ClientTunnel;

        conf: Conf,
        alloc: Allocator,
        ifce: *NetInterface(T),
        mqtt: *Mqtt(T),
        
        id: []u8,
        bind_addr: u32,
        up_topic_cstr: [:0]u8,
        dn_topic_cstr: [:0]u8,

        pub fn create(alloc: Allocator, ifce: *NetInterface(T), broker: *Mqtt(T), conf: Conf) !*Self {
            const self = try alloc.create(Self);
            self.conf = conf;
            self.alloc = alloc;
            self.ifce = ifce;
            self.mqtt = broker;
            self.bind_addr = (try Ip4Address.parse(conf.bind_addr, 0)).sa.addr;
            self.id = try alloc.alloc(u8, conf.id_length);
            
            std.crypto.random.bytes(self.id);
            const b64_len = Base64UrlEncoder.calcSize(conf.id_length);
            const b64_id = try alloc.alloc(u8, b64_len);
            _ = Base64UrlEncoder.encode(b64_id, self.id);

            self.up_topic_cstr = try std.cstr.addNullByte(alloc, conf.topic);
            
            const dn_topic = try std.fmt.allocPrint(alloc, "{s}/{s}", .{conf.topic, b64_id});
            self.dn_topic_cstr = try std.cstr.addNullByte(alloc, dn_topic);
            try self.mqtt.subscribe(self.dn_topic_cstr, true);
            
            std.log.info("Tunnel: {s} -> {s} ({s})", .{conf.bind_addr, conf.topic, b64_id});
            return self;
        }

        pub fn up(self: *Self, pkt: [] align(@alignOf(IpHeader)) u8) !bool {
            const hdr = @ptrCast(*IpHeader, pkt);
            if(hdr.dst != self.bind_addr) return false;
            const payload = try self.alloc.alloc(u8, self.id.len + pkt.len);
            defer self.alloc.free(payload);
            std.mem.copy(u8, payload, self.id);
            std.mem.copy(u8, payload[self.id.len..], pkt);
            try self.mqtt.send(self.up_topic_cstr, payload);
            return true;
        }

        pub fn down(self: *Self, topic: []const u8, msg: [] align(@alignOf(IpHeader)) u8) !bool {
            if(!std.mem.eql(u8, topic, self.dn_topic_cstr)) return false;
            try self.ifce.inject(self.bind_addr, msg);
            return true;
        }
    };
}

pub const Client = struct {
    const Self = @This();
    const Error = error {
        ConfigMissing
    };

    arena: ArenaAllocator,
    alloc: Allocator,
    ifce: ?*NetInterface(*Self),
    mqtt: ?*Mqtt(*Self),
    tunnels: []*Tunnel(*Self),

    connect_callback: ?ConnectHandler(*Mqtt(*Self)),
    disconnect_callback: ?ConnectHandler(*Mqtt(*Self)),
    message_hook: ?MessageHook(*Mqtt(*Self)),

    pub fn init(parent_alloc: Allocator, conf: * const Config) !*Self {
        const client_conf = conf.client orelse {
            std.log.err("missing client config", .{});
            return Error.ConfigMissing;
        };
        const self = try parent_alloc.create(Self);
        self.arena = ArenaAllocator.init(parent_alloc);
        self.alloc = self.arena.allocator();
        self.ifce = null;
        self.mqtt = null;
        errdefer self.deinit();

        std.log.info("== Client Config =================================", .{});
        self.ifce = try NetInterface(*Self).init(self.alloc, conf, self, @ptrCast(driver.PacketHandler(*Self), &up));
        const max_subs = client_conf.tunnels.len;
        self.mqtt = try Mqtt(*Self).init(self.alloc, conf, self, @ptrCast(mqtt.PacketHandler(*Self), &down), max_subs);
        self.tunnels = try self.alloc.alloc(*Tunnel(*Self), client_conf.tunnels.len);
        for (client_conf.tunnels) |tunnel, idx| {
            self.tunnels[idx] = try Tunnel(*Self).create(
                self.alloc,
                self.ifce orelse unreachable,
                self.mqtt orelse unreachable,
                .{
                    .id_length = tunnel.id_length,
                    .topic = tunnel.topic,
                    .bind_addr = tunnel.bind_addr,
                }
            );
        }
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

    pub fn setConnectCallback(self: *Self, cb: ConnectHandler(*Mqtt(*Self))) void {
        self.connect_callback = cb;
        self.mqtt.?.setConnectCallback(&onConnect);
    }
    fn onConnect(self: *Self, idx: usize, count: u32) void {
        self.connect_callback.?(self.mqtt.?, idx, count);
    }

    pub fn setDisconnectCallback(self: *Self, cb: ConnectHandler(*Mqtt(*Self))) void {
        self.disconnect_callback = cb;
        self.mqtt.?.setDisconnectCallback(&onDisconnect);
    }
    fn onDisconnect(self: *Self, idx: usize, count: u32) void {
        self.connect_callback.?(self.mqtt.?, idx, count);
    }

    pub fn attachMsgHook(self: *Self, cb: MessageHook(*Mqtt(*Self))) void {
        self.message_hook = cb;
    }

    fn up(self: *Self, pkt: [] align(@alignOf(IpHeader)) u8) void {
        for (self.tunnels) |tunnel| {
            const handled = tunnel.up(pkt) catch |err| blk: {
                std.log.warn("up: {}", .{err});
                break :blk false;
            };
            if(handled) break;
        }
    }

    fn down(self: *Self, topic: []const u8, msg: [] align(@alignOf(IpHeader)) u8) void {
        for (self.tunnels) |tunnel| {
            const handled = tunnel.down(topic, msg) catch |err| blk: {
                std.log.warn("down: {}", .{err});
                break :blk false;
            };
            if(handled) break;
        }
        if(self.message_hook) |cb| {
            if(cb(self.mqtt.?, topic, msg)) self.message_hook = null;
        }
    }
};

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    var alloc = gpa.allocator();
    const conf_path = try std.fs.cwd().realpathAlloc(alloc, "zika_config.json");
    const conf = try config.get(alloc, conf_path);
    const client = try Client.init(alloc, &conf);
    defer client.deinit();
    defer alloc.destroy(client);
    try client.run();
}
