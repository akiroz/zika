const builtin = @import("builtin");
const std = @import("std");
const Compile = std.Build.Step.Compile;

fn commonOpts(exe: *Compile) *Compile {
    if (builtin.target.isDarwin()) { // macOS
        if (builtin.cpu.arch == .aarch64) { // Apple
            exe.addIncludePath(.{ .path = "/opt/homebrew/include"});
            exe.addLibraryPath(.{ .path = "/opt/homebrew/lib"});
        } else { // Intel
            exe.addIncludePath(.{ .path = "/usr/local/include"});
            exe.addLibraryPath(.{ .path = "/usr/local/lib"});
        }
        exe.linkSystemLibrary("pcap");
    } else { // Linux
        exe.addIncludePath(.{ .path = "/usr/include"});
        exe.addIncludePath(.{ .path = "/usr/include/x86_64-linux-gnu"});
        exe.addLibraryPath(.{ .path = "/usr/lib"});
        exe.addLibraryPath(.{ .path = "/usr/lib/x86_64-linux-gnu"});
        exe.linkLibC();
    }
    exe.linkSystemLibrary("mosquitto");
    return exe;
}
pub fn build(b: *std.build.Builder) void {
    const server = commonOpts(b.addExecutable(.{
        .name = "zika-server",
        .root_source_file = .{ .path = "src/server.zig" },
        // Support down to Ubuntu 20 Focal
        .target = if (builtin.target.isGnuLibC()) .{ .glibc_version = .{ .major = 2, .minor = 31, .patch = 0 } } else .{},
    }));
    b.installArtifact(server);

    const client = commonOpts(b.addExecutable(.{
        .name = "zika-client",
        .root_source_file = .{ .path = "src/client.zig" },
        // Support down to Ubuntu 20 Focal
        .target = if (builtin.target.isGnuLibC()) .{ .glibc_version = .{ .major = 2, .minor = 31, .patch = 0 } } else .{},
    }));
    b.installArtifact(client);
}
