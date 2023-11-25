const builtin = @import("builtin");
const std = @import("std");
const Config = @import("config.zig").Config;

const Net = @cImport({
    @cInclude("sys/ioctl.h");
    @cInclude("net/if.h");
    @cInclude("linux/if_tun.h");
});
const Pcap = @cImport(@cInclude("pcap/pcap.h"));

const Allocator = std.mem.Allocator;
const Ip4Address = std.net.Ip4Address;
const Address = std.net.Address;
pub fn PacketHandler(comptime T: type) type {
    return *const fn (T, []const u8) void;
}

fn PcapDriver(comptime T: type) type {
    return struct {
        const Self = @This();
        const Error = error{
            ConfigMissing,
            CreateFailed,
            ActivateFailed,
            CompileFilterFailed,
            SetFilterFailed,
            InjectFailed,
            ReadFailed,
            NotInitialized,
        };

        alloc: Allocator,
        user: T,
        handler: PacketHandler(T),
        pcap: ?*Pcap.pcap_t,
        pcap_header: [4]u8,

        pub fn init(alloc: Allocator, conf: *const Config, user: T, handler: PacketHandler(T)) !*Self {
            const pcap_conf = conf.driver.pcap orelse {
                std.log.err("missing pcap driver config", .{});
                return Error.ConfigMissing;
            };

            const self = try alloc.create(Self);
            self.alloc = alloc;
            self.user = user;
            self.handler = handler;
            self.pcap_header = .{ 2, 0, 0, 0 };

            const interface = try alloc.dupeZ(u8, pcap_conf.interface);
            var pcap_err = std.mem.zeroes([Pcap.PCAP_ERRBUF_SIZE]u8);
            const pcap = Pcap.pcap_create(@as([*c]const u8, @ptrCast(interface)), &pcap_err) orelse {
                std.log.err("pcap_create: {s}", .{pcap_err});
                return Error.CreateFailed;
            };
            self.pcap = pcap;
            errdefer self.deinit();

            _ = Pcap.pcap_set_promisc(pcap, 1);
            _ = Pcap.pcap_set_immediate_mode(pcap, 1);
            if (Pcap.pcap_activate(pcap) < 0) {
                std.log.err("pcap_activate: {s}", .{Pcap.pcap_geterr(pcap)});
                return Error.ActivateFailed;
            }

            const filter_spec = try std.fmt.allocPrint(alloc, "ip and not dst host {s}", .{conf.driver.local_addr});
            const filter_cstr = try alloc.dupeZ(u8, filter_spec);
            var filter: Pcap.bpf_program = undefined;
            if (Pcap.pcap_compile(pcap, &filter, @as([*c]const u8, @ptrCast(filter_cstr)), 1, Pcap.PCAP_NETMASK_UNKNOWN) < 0) {
                std.log.err("pcap_compile: {s}", .{Pcap.pcap_geterr(pcap)});
                return Error.CompileFilterFailed;
            }
            defer Pcap.pcap_freecode(&filter);

            if (Pcap.pcap_setfilter(pcap, &filter) < 0) {
                std.log.err("pcap_setfilter: {s}", .{Pcap.pcap_geterr(pcap)});
                return Error.SetFilterFailed;
            }

            std.log.info("Driver: pcap", .{});
            return self;
        }

        pub fn deinit(self: *Self) void {
            if (self.pcap) |pcap| Pcap.pcap_close(pcap);
        }

        pub fn run(self: *Self) !void {
            if (self.pcap) |pcap| {
                if (Pcap.pcap_loop(pcap, -1, recv, @as([*c]u8, @ptrCast(self))) == Pcap.PCAP_ERROR) {
                    std.log.err("pcap_loop: {s}", .{Pcap.pcap_geterr(pcap)});
                    return Error.ReadFailed;
                }
            } else {
                return Error.NotInitialized;
            }
        }

        fn recv(self_ptr: [*c]u8, hdr: [*c]const Pcap.pcap_pkthdr, pkt: [*c]const u8) callconv(.C) void {
            const self = @as(*Self, @ptrCast(@alignCast(self_ptr.?)));
            self.handler(self.user, pkt[4..hdr.*.len]);
        }

        pub fn write(self: *Self, pkt: []u8) !void {
            if (self.pcap) |pcap| {
                const frame = try self.alloc.alloc(u8, pkt.len + 4);
                defer self.alloc.free(frame);
                std.mem.copy(u8, frame, self.pcap_header[0..]);
                std.mem.copy(u8, frame[4..], pkt);
                if (Pcap.pcap_inject(pcap, frame.ptr, frame.len) == Pcap.PCAP_ERROR) {
                    std.log.err("pcap_inject: {s}", .{Pcap.pcap_geterr(pcap)});
                    return Error.InjectFailed;
                }
            } else {
                return Error.NotInitialized;
            }
        }
    };
}

fn TunDriver(comptime T: type) type {
    return struct {
        const Self = @This();
        const Error = error{
            ConfigMissing,
            IoctlFailed,
            ReadFailed,
        };

        user: T,
        handler: PacketHandler(T),
        tun: ?std.fs.File = null,
        buf: []u8,

        pub fn init(alloc: Allocator, conf: *const Config, user: T, handler: PacketHandler(T)) !*Self {
            const tun_conf = conf.driver.tun orelse {
                std.log.err("missing tun driver config", .{});
                return Error.ConfigMissing;
            };
            const ip = Address{ .in = try Ip4Address.parse(conf.driver.local_addr, 0) };
            const mask = Address{ .in = try Ip4Address.parse(tun_conf.netmask, 0) };

            const self = try alloc.create(Self);
            self.user = user;
            self.handler = handler;
            self.tun = try std.fs.openFileAbsolute("/dev/net/tun", .{ .mode = .read_write });
            errdefer self.deinit();
            self.buf = try alloc.alloc(u8, 1 << 16); // 64k

            var err: c_int = 0;
            var ifreq = std.mem.zeroes(std.os.ifreq);

            ifreq.ifru.flags = Net.IFF_TUN | Net.IFF_NO_PI;
            err = std.c.ioctl(self.tun.?.handle, Net.TUNSETIFF, &ifreq);
            if (err < 0) {
                std.log.err("ioctl TUNSETIFF: {d}", .{std.c.getErrno(err)});
                return Error.IoctlFailed;
            }

            const sockfd = try std.os.socket(Net.AF_INET, Net.SOCK_DGRAM, 0);
            defer std.os.close(sockfd);

            ifreq.ifru.addr = ip.any;
            err = std.c.ioctl(sockfd, Net.SIOCSIFADDR, &ifreq);
            if (err < 0) {
                std.log.err("ioctl SIOCSIFADDR: {d}", .{std.c.getErrno(err)});
                return Error.IoctlFailed;
            }

            ifreq.ifru.netmask = mask.any;
            err = std.c.ioctl(sockfd, Net.SIOCSIFNETMASK, &ifreq);
            if (err < 0) {
                std.log.err("ioctl SIOCSIFNETMASK: {d}", .{std.c.getErrno(err)});
                return Error.IoctlFailed;
            }

            ifreq.ifru.mtu = 1500;
            err = std.c.ioctl(sockfd, Net.SIOCSIFMTU, &ifreq);
            if (err < 0) {
                std.log.err("ioctl SIOCSIFMTU: {d}", .{std.c.getErrno(err)});
                return Error.IoctlFailed;
            }

            err = std.c.ioctl(sockfd, Net.SIOCGIFFLAGS, &ifreq);
            ifreq.ifru.flags |= @as(i16, Net.IFF_UP | Net.IFF_RUNNING);
            ifreq.ifru.flags &= @as(i16, ~Net.IFF_MULTICAST);
            err = std.c.ioctl(sockfd, Net.SIOCSIFFLAGS, &ifreq);
            if (err < 0) {
                std.log.err("ioctl SIOCSIFFLAGS: {d}", .{std.c.getErrno(err)});
                return Error.IoctlFailed;
            }

            std.log.info("Driver: tun", .{});
            return self;
        }

        pub fn deinit(self: *Self) void {
            self.tun.?.close();
        }

        pub fn run(self: *Self) !void {
            while (true) {
                const len = try self.tun.?.read(self.buf);
                if (len == 0) return Error.ReadFailed;
                self.handler(self.user, self.buf[0..len]);
            }
        }

        pub fn write(self: *Self, pkt: []u8) !void {
            _ = try self.tun.?.write(pkt);
        }
    };
}

pub const IpHeader = packed struct {
    ihl: u4,
    ver: u4,
    tos: u8,
    len: u16,
    id: u16,
    frag_off: u16,
    ttl: u8,
    proto: u8,
    cksum: u16,
    src: u32,
    dst: u32,
};

const PseudoHeader = packed struct {
    src: u32,
    dst: u32,
    zero: u8 = 0,
    proto: u8,
    len: u16,
};

pub fn NetInterface(comptime T: type) type {
    const Driver = if (builtin.target.isDarwin()) PcapDriver else TunDriver;

    return struct {
        const Self = @This();
        alloc: Allocator,
        local_ip: u32,
        driver: ?*Driver(T) = null,

        pub fn init(alloc: Allocator, conf: *const Config, user: T, handler: PacketHandler(T)) !*Self {
            const self = try alloc.create(Self);
            errdefer self.deinit();
            self.alloc = alloc;
            self.local_ip = (try Ip4Address.parse(conf.driver.local_addr, 0)).sa.addr;
            self.driver = try Driver(T).init(alloc, conf, user, handler);
            std.log.info("Local IP: {s}", .{conf.driver.local_addr});
            return self;
        }

        pub fn deinit(self: *Self) void {
            self.driver.?.deinit();
        }

        pub fn run(self: *Self) !void {
            try self.driver.?.run();
        }

        fn cksum(buf: []align(@alignOf(u16)) u8, carry: u32) !u16 {
            var sum: u32 = carry;
            const buf16: []u16 = @as([*]u16, @ptrCast(buf.ptr))[0 .. buf.len / 2];
            for (buf16) |word| sum += word;
            if (buf.len % 2 == 1) sum += buf[buf.len - 1];
            while (sum > 0xffff) sum = (sum & 0xffff) + (sum >> 16);
            return @truncate(sum ^ 0xffff);
        }

        pub fn inject(self: *Self, src: u32, pkt: []align(@alignOf(IpHeader)) u8) !void {
            const hdr = @as(*IpHeader, @ptrCast(pkt));
            const payload_offset = @as(u16, @intCast(hdr.ihl)) * 4;
            hdr.src = src;
            hdr.dst = self.local_ip;
            hdr.cksum = 0; // Zero before recalc
            hdr.cksum = try cksum(pkt[0..payload_offset], 0);
            switch (hdr.proto) {
                6, 17 => { // TCP / UDP
                    var pseudo_buf align(@alignOf(IpHeader)) = std.mem.zeroes([@sizeOf(PseudoHeader) / 2]u16);
                    const pseudo_hdr = @as(*PseudoHeader, @ptrCast(&pseudo_buf));
                    pseudo_hdr.src = src;
                    pseudo_hdr.dst = self.local_ip;
                    pseudo_hdr.proto = hdr.proto;
                    pseudo_hdr.len = std.mem.nativeToBig(u16, @as(u16, @intCast(pkt.len)) - payload_offset);
                    var pseudo_sum: u32 = 0;
                    for (pseudo_buf) |word| pseudo_sum += word;
                    const cksum_offset = payload_offset + if (hdr.proto == 6) @as(usize, 16) else 6;
                    const cksum_slice = pkt[cksum_offset .. cksum_offset + 2];
                    @memset(cksum_slice, 0); // Zero before recalc
                    var sum = try cksum(@alignCast(pkt[payload_offset..]), pseudo_sum);
                    @memcpy(cksum_slice, @as(*[2]u8, @ptrCast(&sum)));
                },
                else => {}, // No special handling
            }
            try self.driver.?.write(pkt);
        }
    };
}
