const std = @import("std");
const config = @import("config.zig");

const Mosq = @cImport({
    @cInclude("mqtt_protocol.h");
    @cInclude("mosquitto.h");
});

const Allocator = std.mem.Allocator;
const Config = config.Config;

pub fn PacketHandler(comptime T: type) type {
    return *const fn(T, []const u8) void;
}

const Client = struct {
    const Self = @This();
    const Opts = config.MqttOptions;
    const Error = error {
        CreateFailed,
    };

    mosq: *Mosq.mosquitto,
    
    pub fn init(alloc: *Allocator, url: []const u8, opts: Opts) !*Self {
        const self = try alloc.create(Self);
        errdefer alloc.destroy(self);

        self.mosq = Mosq.mosquitto_new(null, true, self) orelse {
            return Error.CreateFailed;
        };
        
        return self;
    }

};

pub fn Mqtt(comptime T: type) type {
    return struct {
        const Self = @This();

        clients: []*Client,

        pub fn init(alloc: *Allocator, conf: * const Config, user: T, handler: PacketHandler(T)) !*Self {

            const self = try alloc.create(Self);
            errdefer alloc.destroy(self);

            self.clients = try alloc.alloc(*Client, conf.mqtt.brokers.len);
            errdefer alloc.free(self.clients);

            for (conf.mqtt.brokers) |broker, idx| {
                const opts =  broker.options orelse conf.mqtt.options;
                self.clients[idx] = try Client.init(alloc, broker.url, opts);
            }

            return self;
        }

        pub fn send(self: *Self, topic: []u8, msg: []u8) !void {

        }
    };
}