const builtin = @import("builtin");
const Builder = @import("std").build.Builder;

pub fn build(b: *Builder) void {
    const server = b.addExecutable("zika-server", "src/server.zig");
    const client = b.addExecutable("zika-client", "src/client.zig");
    server.addLibPath(".");
    client.addLibPath(".");
    server.linkSystemLibrary("paho-mqtt3as.1");
    client.linkSystemLibrary("paho-mqtt3as.1");
    if (builtin.target.isDarwin()) {
        server.linkSystemLibrary("pcap");
        client.linkSystemLibrary("pcap");
    }
    //server.install();
    client.install();
}
