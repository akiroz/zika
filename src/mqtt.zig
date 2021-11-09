const Paho = @cImport(@cInclude("paho/MQTTAsync.h"));

pub const Mqtt = struct {
    const Self = @This();
    const Error = error {
        CreateFailed,
    };

    client: Paho.MQTTAsync,

    pub fn init(alloc: *Allocator, conf: * const Config) !*Self {
        
        const self = try alloc.create(Self);
        errdefer alloc.destroy(self);
        
        var err = Paho.MQTTAsync_create(&self.client, "", "a", Paho.MQTTCLIENT_PERSISTENCE_NONE, null);
        if(err != Paho.MQTTASYNC_SUCCESS) {
            std.log.err("MQTTAsync_create: {s}", .{Paho.MQTTAsync_strerror(err)});
            return Error.CreateFailed;
        }

        err = Paho.MQTTAsync_connect(&self.client, );

        return self;
    }

};
