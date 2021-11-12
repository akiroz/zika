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
        OptionError,
        ConnectFailed,
    };

    mosq: *Mosq.mosquitto,
    
    pub fn init(alloc: *Allocator, host: []const u8, port: u16, opts: Opts) !*Self {
        const self = try alloc.create(Self);
        errdefer alloc.destroy(self);

        self.mosq = Mosq.mosquitto_new(null, true, self) orelse {
            return Error.CreateFailed;
        };
        
        var rc = Mosq.mosquitto_reconnect_delay_set(self.mosq, opts.reconnect_interval_min, opts.reconnect_interval_max, true);
        if(rc != Mosq.MOSQ_ERR_SUCCESS) {
            std.log.err("mosquitto_reconnect_delay_set: {s}", .{Mosq.mosquitto_strerror(rc)});
            return Error.OptionError;
        }

        if(opts.username) |username| {
            const username_str = try std.cstr.addNullByte(alloc, username);
            errdefer alloc.free(username_str);
            const password_str: [*c]const u8 = if(opts.password) |password| blk: {
                const str = try std.cstr.addNullByte(alloc, password);
                errdefer alloc.free(str);
                break :blk str;
            } else null;
            rc = Mosq.mosquitto_username_pw_set(self.mosq, username_str, password_str);
            if(rc != Mosq.MOSQ_ERR_SUCCESS) {
                std.log.err("mosquitto_username_pw_set: {s}", .{Mosq.mosquitto_strerror(rc)});
                return Error.OptionError;
            }
        }

        if(opts.ca != null or opts.cert != null) {
            const ca_path: [*c]const u8 = if(opts.ca) |ca| blk: {
                const str = try std.cstr.addNullByte(alloc, ca);
                errdefer alloc.free(str);
                break :blk str;
            } else null;

            const key_path: [*c]const u8 = if(opts.key) |key| blk: {
                const str = try std.cstr.addNullByte(alloc, key);
                errdefer alloc.free(str);
                break :blk str;
            } else null;

            const cert_path: [*c]const u8 = if(opts.cert) |cert| blk: {
                const str = try std.cstr.addNullByte(alloc, cert);
                errdefer alloc.free(str);
                break :blk str;
            } else null;

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

        const host_str = try std.cstr.addNullByte(alloc, host);
        errdefer alloc.free(host_str);
        rc = Mosq.mosquitto_connect_async(self.mosq, host_str, port, opts.keepalive_interval);
        if(rc != Mosq.MOSQ_ERR_SUCCESS) {
            std.log.err("mosquitto_connect_async: {s}", .{Mosq.mosquitto_strerror(rc)});
            return Error.ConnectFailed;
        }

        rc = Mosq.mosquitto_loop_start(self.mosq);
        if(rc != Mosq.MOSQ_ERR_SUCCESS) {
            std.log.err("mosquitto_loop_start: {s}", .{Mosq.mosquitto_strerror(rc)});
            return Error.ConnectFailed;
        }
        
        return self;
    }

    fn onConnect(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, rc: c_int) callconv(.C) void {
        const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));

    }

    fn onDisconnect(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, rc: c_int) callconv(.C) void {
        const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));

    }

    fn onSubscribe(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, mid: c_int, qos_len: c_int, qos: [*c]const c_int) callconv(.C) void {
        const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));
    }

    fn onMessage(_mosq: ?*Mosq.mosquitto, self_ptr: ?*c_void, msg: [*c]const Mosq.mosquitto_message) callconv(.C) void {
        const self = @intToPtr(*Self, @ptrToInt(self_ptr orelse unreachable));

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
                self.clients[idx] = try Client.init(alloc, broker.host, broker.port, opts);
            }

            return self;
        }

        pub fn send(self: *Self, topic: []u8, msg: []u8) !void {

        }
    };
}