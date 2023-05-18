const builtin = @import("builtin");
const std = @import("std");
const Compile = std.Build.Step.Compile;

fn commonOpts(exe: *Compile) *Compile {
    if (builtin.target.isDarwin()) { // macOS
        if (builtin.cpu.arch == .aarch64) { // Apple
            exe.addIncludePath("/opt/homebrew/include");
            exe.addLibraryPath("/opt/homebrew/lib");
        } else { // Intel
            exe.addIncludePath("/usr/local/include");
            exe.addLibraryPath("/usr/local/lib");
        }
        exe.linkSystemLibrary("pcap");
    } else { // Linux
        exe.addIncludePath("/usr/include");
        exe.addIncludePath("/usr/include/x86_64-linux-gnu");
        exe.addLibraryPath("/usr/lib");
        exe.addLibraryPath("/usr/lib/x86_64-linux-gnu");
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
