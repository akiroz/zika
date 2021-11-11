const builtin = @import("builtin");
const Builder = @import("std").build.Builder;

pub fn build(b: *Builder) void {
    const server = b.addExecutable("zika-server", "src/server.zig");
    const client = b.addExecutable("zika-client", "src/client.zig");
    server.addIncludeDir("/usr/local/include");
    client.addIncludeDir("/usr/local/include");
    server.linkSystemLibrary("mosquitto");
    client.linkSystemLibrary("mosquitto");
    if (builtin.target.isDarwin()) {
        server.linkSystemLibrary("pcap");
        client.linkSystemLibrary("pcap");
    }
    //server.install();
    client.install();
}
