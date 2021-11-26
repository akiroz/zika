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
    // user, message
    return *const fn (T, []const u8, []const u8) void;
}

pub fn ConnectHandler(comptime T: type) type {
    // mqtt_client, client_idx, connect_count
    return *const fn (T, usize, u32) void;
}

pub fn Client(comptime T: type) type {
    return struct {
        const Self = @This();
        const Conf = struct {
            nth: usize,
            host: []const u8,
            port: u16,
            opts: config.MqttOptions,
        };
        const Error = error{
            CreateFailed,
            OptionError,
            ConnectFailed,
            SubscribeFailed,
        };

        alloc: *Allocator,
        mosq: *Mosq.mosquitto,

        conf: Conf,
        host_cstr: []const u8,
        subs: [][]u8,
        subs_count: u32,
        user: T,
        msg_callback: PacketHandler(T),
        connect_count: u32,
        connect_callback: ?ConnectHandler(T),
        disconnect_callback: ?ConnectHandler(T),
        subscribe_cond: std.Thread.Condition,

        pub fn init(alloc: *Allocator, conf: Conf, user: T, handler: PacketHandler(T), max_subs: usize) !*Self {
            const self = try alloc.create(Self);
            self.alloc = alloc;
            self.conf = conf;
            self.host_cstr = try std.cstr.addNullByte(alloc, conf.host);
            self.subs_count = 0;
            self.subs = try alloc.alloc([:0]u8, max_subs);
            self.user = user;
            self.msg_callback = handler;
            self.connect_count = 0;
            self.connect_callback = null;
            self.disconnect_callback = null;
            self.subscribe_cond = std.Thread.Condition {};
            self.mosq = Mosq.mosquitto_new(null, true, self) orelse {
                return Error.CreateFailed;
            };
            errdefer self.deinit();

            const opts = conf.opts;
            var rc = Mosq.mosquitto_reconnect_delay_set(self.mosq, opts.reconnect_interval_min, opts.reconnect_interval_max, true);
            if (rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_reconnect_delay_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                return Error.OptionError;
            }

            if (opts.username) |username| {
                const username_str = try std.cstr.addNullByte(alloc, username);
                const password_str: [*c]const u8 = if (opts.password) |password| (try std.cstr.addNullByte(alloc, password)) else null;
                rc = Mosq.mosquitto_username_pw_set(self.mosq, username_str, password_str);
                if (rc != Mosq.MOSQ_ERR_SUCCESS) {
                    std.log.err("mosquitto_username_pw_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                    return Error.OptionError;
                }
            }

            if (opts.ca != null or opts.cert != null) {
                const ca_path: [*c]const u8 = if (opts.ca) |ca| (try std.cstr.addNullByte(alloc, ca)) else null;
                const key_path: [*c]const u8 = if (opts.key) |key| (try std.cstr.addNullByte(alloc, key)) else null;
                const cert_path: [*c]const u8 = if (opts.cert) |cert| (try std.cstr.addNullByte(alloc, cert)) else null;

                rc = Mosq.mosquitto_tls_set(self.mosq, ca_path, null, cert_path, key_path, null);
                if (rc != Mosq.MOSQ_ERR_SUCCESS) {
                    std.log.err("mosquitto_tls_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                    return Error.OptionError;
                }

                if (opts.tls_insecure) {
                    rc = Mosq.mosquitto_tls_insecure_set(self.mosq, true);
                    if (rc != Mosq.MOSQ_ERR_SUCCESS) {
                        std.log.err("mosquitto_tls_insecure_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                        return Error.OptionError;
                    }
                }
            }

            Mosq.mosquitto_connect_callback_set(self.mosq, onConnect);
            Mosq.mosquitto_disconnect_callback_set(self.mosq, onDisconnect);
            Mosq.mosquitto_subscribe_callback_set(self.mosq, onSubscribe);
            Mosq.mosquitto_message_callback_set(self.mosq, onMessage);
            // Mosq.mosquitto_log_callback_set(self.mosq, onLog);

            std.log.info("MQTT[{d}]: {s}:{d}", .{ conf.nth, conf.host, conf.port });
            return self;
        }

        pub fn deinit(self: *Self) void {
            Mosq.mosquitto_destroy(self.mosq);
        }

        pub fn connect(self: *Self) !void {
            var rc = Mosq.mosquitto_loop_start(self.mosq);
            if (rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_loop_start: {s}", .{Mosq.mosquitto_strerror(rc)});
                return Error.ConnectFailed;
            }

            const keepalive = self.conf.opts.keepalive_interval;
            rc = Mosq.mosquitto_connect_async(self.mosq, @ptrCast([*c]const u8, self.host_cstr), self.conf.port, keepalive);
            if (rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_connect_async: {s}", .{Mosq.mosquitto_strerror(rc)});
                return Error.ConnectFailed;
            }
        }

        fn onConnect(_: ?*Mosq.mosquitto, self_ptr: ?*c_void, rc: c_int) callconv(.C) void {
            const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));
            std.log.info("connect[{d}]: {s}", .{ self.conf.nth, Mosq.mosquitto_strerror(rc) });
            
            self.connect_count += 1;
            if (self.connect_callback) |cb| {
                cb.*(self.user, self.conf.nth, self.connect_count);
            }

            var i: usize = 0;
            while(i < self.subs_count) : (i += 1) {
                const sub_rc = Mosq.mosquitto_subscribe(self.mosq, null, self.subs[i].ptr, 0);
                if(sub_rc != Mosq.MOSQ_ERR_SUCCESS and rc != Mosq.MOSQ_ERR_NO_CONN) {
                    std.log.err("mosquitto_subscribe[{d}]: {s}", .{ self.conf.nth, Mosq.mosquitto_strerror(sub_rc) });
                }
            }
        }

        fn onDisconnect(_: ?*Mosq.mosquitto, self_ptr: ?*c_void, rc: c_int) callconv(.C) void {
            const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));
            std.log.info("disconnect[{d}]: {s}", .{ self.conf.nth, Mosq.mosquitto_strerror(rc) });
            if (self.disconnect_callback) |cb| {
                cb.*(self.user, self.conf.nth, self.connect_count);
            }
        }

        fn onSubscribe(_: ?*Mosq.mosquitto, self_ptr: ?*c_void, _mid: c_int, qos_len: c_int, qos_arr: [*c]const c_int) callconv(.C) void {
            const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));
            const qos = qos_arr[0..@intCast(usize, qos_len)];
            for(qos) |q| if(q != 0) std.log.warn("subscribe[{d}]: {d}", .{ self.conf.nth, 1 });
            self.subscribe_cond.broadcast();
        }

        fn onMessage(_: ?*Mosq.mosquitto, self_ptr: ?*c_void, msg: [*c]const Mosq.mosquitto_message) callconv(.C) void {
            const topic = msg.*.topic[0..std.mem.len(msg.*.topic)];
            // std.log.info("message: {s}", .{topic});
            const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));
            const len = @intCast(usize, msg.*.payloadlen);
            const payload = @ptrCast([*c]const u8, msg.*.payload)[0..len];
            self.msg_callback.*(self.user, topic, payload);
        }

        fn onLog(_: ?*Mosq.mosquitto, _: ?*c_void, level: c_int, msg: [*c]const u8) callconv(.C) void {
            std.log.info("mosq({d}): {s}", .{level, msg});
        }

        pub fn subscribe(self: *Self, topic: [:0]u8, persistent: bool) !void {
            if(persistent) {
                if (self.subs_count < self.subs.len) {
                    self.subs[self.subs_count] = topic;
                    self.subs_count += 1;
                } else {
                    std.log.err("subscribe[{d}]: persistent subscription list is full", .{ self.conf.nth });
                    return Error.SubscribeFailed;
                }
            } else {
                const rc = Mosq.mosquitto_subscribe(self.mosq, null, topic.ptr, 0);
                if(rc != Mosq.MOSQ_ERR_SUCCESS and rc != Mosq.MOSQ_ERR_NO_CONN) {
                    std.log.err("mosquitto_subscribe[{d}]: {s}", .{ self.conf.nth, Mosq.mosquitto_strerror(rc) });
                    return Error.SubscribeFailed;
                }
            }
        }

        // NOTE: topic must be null-terminated
        pub fn unsubscribe(self: *Self, topic: [:0]u8) void {
            const rc = Mosq.mosquitto_unsubscribe(self.mosq, null, topic.ptr);
            if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_unsubscribe[{d}]: {s}", .{ self.conf.nth, Mosq.mosquitto_strerror(rc) });
            }
        }

        // NOTE: topic must be null-terminated
        pub fn publish(self: *Self, topic: [:0]u8, msg: []u8) bool {
            const rc = Mosq.mosquitto_publish(self.mosq, null, topic.ptr, @intCast(c_int, msg.len), msg.ptr, 0, false);
            return rc == Mosq.MOSQ_ERR_SUCCESS;
        }
    };
}

pub fn Mqtt(comptime T: type) type {
    return struct {
        const Self = @This();

        arena: ArenaAllocator,
        clients: []*Client(T),

        pub fn init(alloc: *Allocator, conf: *const Config, user: T, handler: PacketHandler(T), max_subs: usize) !*Self {
            var arena = ArenaAllocator.init(alloc);
            const self = try arena.allocator.create(Self);
            errdefer self.deinit();

            self.arena = arena;
            self.clients = try arena.allocator.alloc(*Client(T), conf.mqtt.brokers.len);

            for (conf.mqtt.brokers) |broker, idx| {
                const opts = broker.options orelse conf.mqtt.options;
                // idx, broker.host, broker.port, opts
                self.clients[idx] = try Client(T).init(&arena.allocator, .{ .nth = idx, .host = broker.host, .port = broker.port, .opts = opts }, user, handler, max_subs);
            }

            return self;
        }

        pub fn deinit(self: *Self) void {
            for (self.clients) |client| client.deinit();
            self.arena.deinit();
        }

        pub fn setConnectCallback(self: *Self, cb: ConnectHandler(T)) void {
            for (self.clients) |client| client.connect_callback = cb;
        }

        pub fn setDisconnectCallback(self: *Self, cb: ConnectHandler(T)) void {
            for (self.clients) |client| client.disconnect_callback = cb;
        }

        pub fn connect(self: *Self) !void {
            for (self.clients) |client| try client.connect();
        }

        pub fn subscribe(self: *Self, topic: [:0]u8, persistent: bool) !void {
            for (self.clients) |client| try client.subscribe(topic, persistent);
        }

        pub fn unsubscribe(self: *Self, topic: [:0]u8) !void {
            for (self.clients) |client| client.unsubscribe(topic);
        }

        pub fn send(self: *Self, topic: [:0]u8, msg: []u8) !void {
            for (self.clients) |client| {
                if (client.publish(topic, msg)) break;
            }
        }
    };
}
