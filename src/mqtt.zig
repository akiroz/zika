const std = @import("std");
const config = @import("config.zig");
const CString = @cImport(@cInclude("string.h"));
const Mosq = @cImport({
    @cInclude("mqtt_protocol.h");
    @cInclude("mosquitto.h");
});

const Allocator = std.mem.Allocator;
const ArenaAllocator = std.heap.ArenaAllocator;
const Config = config.Config;

pub fn PacketHandler(comptime T: type) type {
    return *const fn(T, []const u8) void;
}

pub fn Client(comptime T: type) type {
    return struct {
        const Self = @This();
        const Opts = config.MqttOptions;
        const Error = error {
            CreateFailed,
            OptionError,
            ConnectFailed,
        };

        mosq: *Mosq.mosquitto,
        host_cstr: []u8,
        port: u16,
        keepalive: u16,
        subscribtions: std.ArrayList([]u8),
        user: T,
        handler: PacketHandler(T),

        pub fn init(alloc: *Allocator, host: []const u8, port: u16, opts: Opts, user: T, handler: PacketHandler(T)) !*Self {
            const self = try alloc.create(Self);
            self.host_cstr = try std.cstr.addNullByte(alloc, host);
            self.port = port;
            self.keepalive = opts.keepalive_interval;
            self.subscribtions = std.ArrayList([]u8).init(alloc);
            self.user = user;
            self.handler = handler;
            self.mosq = Mosq.mosquitto_new(null, true, self) orelse {
                return Error.CreateFailed;
            };
            errdefer self.deinit();

            var rc = Mosq.mosquitto_reconnect_delay_set(self.mosq, opts.reconnect_interval_min, opts.reconnect_interval_max, true);
            if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_reconnect_delay_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                return Error.OptionError;
            }

            if(opts.username) |username| {
                const username_str = try std.cstr.addNullByte(alloc, username);
                const password_str: [*c]const u8 = if(opts.password) |password| (try std.cstr.addNullByte(alloc, password)) else null;
                rc = Mosq.mosquitto_username_pw_set(self.mosq, username_str, password_str);
                if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                    std.log.err("mosquitto_username_pw_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                    return Error.OptionError;
                }
            }

            if(opts.ca != null or opts.cert != null) {
                const ca_path: [*c]const u8 = if(opts.ca) |ca| (try std.cstr.addNullByte(alloc, ca)) else null;
                const key_path: [*c]const u8 = if(opts.key) |key| (try std.cstr.addNullByte(alloc, key)) else null;
                const cert_path: [*c]const u8 = if(opts.cert) |cert| (try std.cstr.addNullByte(alloc, cert)) else null;

                rc = Mosq.mosquitto_tls_set(self.mosq, ca_path, null, cert_path, key_path, null);
                if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                    std.log.err("mosquitto_tls_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                    return Error.OptionError;
                }

                if(opts.tls_insecure) {
                    rc = Mosq.mosquitto_tls_insecure_set(self.mosq, true);
                    if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                        std.log.err("mosquitto_tls_insecure_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                        return Error.OptionError;
                    }
                }
            }

            Mosq.mosquitto_connect_callback_set(self.mosq, onConnect);
            Mosq.mosquitto_disconnect_callback_set(self.mosq, onDisconnect);
            Mosq.mosquitto_subscribe_callback_set(self.mosq, onSubscribe);
            Mosq.mosquitto_message_callback_set(self.mosq, onMessage);

            std.log.info("MQTT: {s}:{d}", .{host, port});
            return self;
        }

        pub fn deinit(self: *Self) void {
            Mosq.mosquitto_destroy(self.mosq);
        }

        pub fn connect(self: *Self) !void {
            var rc = Mosq.mosquitto_loop_start(self.mosq);
            if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_loop_start: {s}", .{Mosq.mosquitto_strerror(rc)});
                return Error.ConnectFailed;
            }

            rc = Mosq.mosquitto_connect_async(self.mosq, @ptrCast([*c]const u8, self.host_cstr), self.port, self.keepalive);
            if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_connect_async: {s}", .{Mosq.mosquitto_strerror(rc)});
                return Error.ConnectFailed;
            }
        }

        fn onConnect(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, rc: c_int) callconv(.C) void {
            const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));
            std.log.info("connect: {s}", .{Mosq.mosquitto_strerror(rc)});
            const len = @intCast(c_int, self.subscribtions.items.len);
            const subs = @ptrCast([*c]const [*c]u8, self.subscribtions.items);
            const sub_rc = Mosq.mosquitto_subscribe_multiple(self.mosq, null, len, subs, 0, 0, null);
            if(sub_rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_subscribe_multiple: {s}", .{Mosq.mosquitto_strerror(rc)});
                std.os.exit(1);
            }
        }

        fn onDisconnect(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, rc: c_int) callconv(.C) void {
            std.log.info("disconnect: {s}", .{Mosq.mosquitto_strerror(rc)});
        }

        fn onSubscribe(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, mid: c_int, qos_len: c_int, qos_arr: [*c]const c_int) callconv(.C) void {
            const qos = qos_arr[0..@intCast(usize, qos_len)];
            std.log.info("subscribe: {d}", .{qos});
        }

        fn onMessage(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, msg: [*c]const Mosq.mosquitto_message) callconv(.C) void {
            std.log.info("message: {s}", .{msg.*.topic});
            const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));
            const len = @intCast(usize, msg.*.payloadlen);
            const payload = @ptrCast([*c]const u8, msg.*.payload)[0..len];
            self.handler.*(self.user, payload);
        }

        // NOTE: topic must be null-terminated
        pub fn publish(self: *Self, topic: []u8, msg: []u8) bool {
            const topic_c = @ptrCast([*c]const u8, topic);
            const rc = Mosq.mosquitto_publish(self.mosq, null, topic_c, @intCast(c_int, msg.len), &msg, 0, false);
            return rc == Mosq.MOSQ_ERR_SUCCESS;
        }

        // NOTE: topic must be null-terminated
        pub fn subscribe(self: *Self, topic: []u8) !void {
            try self.subscribtions.append(topic);
        }

    };
}

pub fn Mqtt(comptime T: type) type {
    return struct {
        const Self = @This();
        
        arena: ArenaAllocator,
        clients: []*Client(T),

        pub fn init(alloc: *Allocator, conf: * const Config, user: T, handler: PacketHandler(T)) !*Self {
            var arena = ArenaAllocator.init(alloc);
            const self = try arena.allocator.create(Self);
            errdefer self.deinit();

            self.arena = arena;
            self.clients = try arena.allocator.alloc(*Client(T), conf.mqtt.brokers.len);

            for (conf.mqtt.brokers) |broker, idx| {
                const opts = broker.options orelse conf.mqtt.options;
                self.clients[idx] = try Client(T).init(&arena.allocator, broker.host, broker.port, opts, user, handler);
            }

            return self;
        }

        pub fn deinit(self: *Self) void {
            for (self.clients) |client| client.deinit();
            self.arena.deinit();
        }

        pub fn connect(self: *Self) !void {
            for (self.clients) |client| try client.connect();
        }

        // NOTE: topic must be null-terminated
        pub fn subscribe(self: *Self, topic: []u8) !void {
            for (self.clients) |client| try client.subscribe(topic);
        }

        // NOTE: topic must be null-terminated
        pub fn send(self: *Self, topic: []u8, msg: []u8) !void {
            for (self.clients) |client| {
                if(client.publish(topic, msg)) break;
            }
        }

    };
}