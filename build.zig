const builtin = @import("builtin");
const std = @import("std");
const LibExeObjStep = std.build.LibExeObjStep;

fn commonOpts(exe: *LibExeObjStep) *LibExeObjStep {
    exe.setBuildMode(exe.builder.standardReleaseOptions());
    if (builtin.target.isDarwin()) { // macOS
        if (builtin.cpu.arch == .aarch64) { // Apple
            exe.addIncludeDir("/opt/homebrew/include");
            exe.addLibPath("/opt/homebrew/lib");
        } else { // Intel
            exe.addIncludeDir("/usr/local/include");
            exe.addLibPath("/usr/local/lib");
        }
        exe.linkSystemLibrary("pcap");
    } else { // Linux
        exe.addIncludeDir("/usr/include");
        exe.addLibPath("/usr/lib");
        exe.linkLibC();
        // Support down to Ubuntu 20 Focal
        exe.setTarget(.{ .glibc_version = .{ .major = 2, .minor = 31, .patch = 0 } });
    }
    exe.linkSystemLibrary("mosquitto");
    return exe;
}

pub fn build(b: *std.build.Builder) void {
    const server = commonOpts(b.addExecutable("zika-server", "src/server.zig"));
    server.install();
    
    const client = commonOpts(b.addExecutable("zika-client", "src/client.zig"));
    client.install();
}