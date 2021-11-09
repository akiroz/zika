const builtin = @import("builtin");
const std = @import("std");
const Config = @import("config.zig").Config;

const Net = @cImport({
    @cInclude("net/if.h");
    @cInclude("net/if_tun.h");
});
const Pcap = @cImport(@cInclude("pcap/pcap.h"));

const Allocator = std.mem.Allocator;
const Ip4Address = std.net.Ip4Address;
pub fn PacketHandler(comptime T: type) type {
    return *const fn(T, []const u8) void;
}

fn PcapDriver(comptime T: type) type {
    return struct {
        const Self = @This();
        const Error = error {
            ConfigMissing,
            CreateFailed,
            ActivateFailed,
            CompileFilterFailed,
            SetFilterFailed,
            InjectFailed,
            ReadFailed,
        };

        user: T,
        handler: PacketHandler(T),
        pcap: *Pcap.pcap_t,
        recv_thread: *std.Thread,

        pub fn init(alloc: *Allocator, conf: *const Config, user: T, handler: PacketHandler(T)) !*Self {
            const pcap_conf = conf.driver.pcap orelse {
                std.log.err("missing pcap driver config", .{});
                return Error.ConfigMissing;
            };

            const interface = try std.cstr.addNullByte(alloc, pcap_conf.interface);
            var pcap_err = std.mem.zeroes([Pcap.PCAP_ERRBUF_SIZE]u8);
            
            const self = try alloc.create(Self);
            errdefer alloc.destroy(self);
            
            self.user = user;
            self.handler = handler;
            self.pcap = Pcap.pcap_create(interface, @ptrCast([*c]u8, &pcap_err)) orelse {
                std.log.err("pcap_create: {s}", .{pcap_err});
                return Error.CreateFailed;
            };
            
            
            _ = Pcap.pcap_set_promisc(self.pcap, 1);
            _ = Pcap.pcap_set_immediate_mode(self.pcap, 1);
            if(Pcap.pcap_activate(self.pcap) < 0) {
                std.log.err("pcap_activate: {s}", .{Pcap.pcap_geterr(self.pcap)});
                return Error.ActivateFailed;
            }
            errdefer Pcap.pcap_close(self.pcap);

            const filter_spec = try std.fmt.allocPrint(alloc, "ip and not dst host {s}", .{conf.driver.local_addr});
            defer alloc.free(filter_spec);
            
            const filter_cstr = try std.cstr.addNullByte(alloc, filter_spec);
            defer alloc.free(filter_cstr);
            
            var filter: Pcap.bpf_program = undefined;
            if(Pcap.pcap_compile(self.pcap, &filter, filter_cstr, 1, Pcap.PCAP_NETMASK_UNKNOWN) < 0) {
                std.log.err("pcap_compile: {s}", .{Pcap.pcap_geterr(self.pcap)});
                return Error.CompileFilterFailed;
            }
            defer Pcap.pcap_freecode(&filter);
            
            if(Pcap.pcap_setfilter(self.pcap, &filter) < 0) {
                std.log.err("pcap_setfilter: {s}", .{Pcap.pcap_geterr(self.pcap)});
                return Error.SetFilterFailed;
            }

            self.recv_thread = try std.Thread.spawn(loop, self);
            std.log.info("Driver: pcap", .{});
            return self;
        }

        fn loop(self: *Self) !void {
            if(Pcap.pcap_loop(self.pcap, -1, recv, @intToPtr([*c]u8, @ptrToInt(self))) == Pcap.PCAP_ERROR) {
                std.log.err("pcap_loop: {s}", .{Pcap.pcap_geterr(self.pcap)});
            }
            return Error.ReadFailed;
        }

        fn recv(self_ptr: [*c]u8, hdr: [*c]const Pcap.pcap_pkthdr, pkt: [*c]const u8) callconv(.C) void {
            const self = @intToPtr(*Self, @ptrToInt(self_ptr));
            self.handler.*(self.user, pkt[0..hdr.*.len]);
        }

        pub fn write(self: *Self, pkt: []u8) !void {
            if(Pcap.pcap_inject(self.pcap, @ptrCast([*c]u8, pkt), pkt.len) == Pcap.PCAP_ERROR) {
                std.log.err("pcap_inject: {s}", .{Pcap.pcap_geterr(self.pcap)});
                return Error.InjectFailed;
            }
        }

    };
}

fn TunDriver(comptime T: type) type {

    return struct {
        const Self = @This();
        const Error = error {
            ConfigMissing,
            IoctlFailed,
        };

        tun: std.fs.File,
        user: T,
        handler: PacketHandler(T),

        pub fn init(alloc: *Allocator, conf: * const Config, user: T, handler: PacketHandler(T)) !*Self {
            const tun_conf = conf.driver.tun orelse {
                std.log.err("missing tun driver config", .{});
                return Error.ConfigMissing;
            };
            const ip = try Ip4Address.parse(conf.driver.local_addr, 0);
            const mask = try Ip4Address.parse(tun_conf.netmask, 0);

            const self = try alloc.create(Self);
            errdefer alloc.destroy(self);

            self.user = user;
            self.handler = handler;
            self.tun = try std.fs.openFileAbsolute("/dev/net/tun", .{ .read = true, .write = true });
            errdefer self.tun.close();

            var err = 0;      
            const ifreq = std.mem.zeroes(std.c.ifreq);

            ifreq.ifru.flags = Net.IFF_TUN | Net.IFF_NO_PI;
            err = std.c.ioctl(self.tun.handle, Net.TUNSETIFF, @ptrCast(c_void, &ifreq));
            if(err < 0) {
                std.log.err("ioctl TUNSETIFF: {d}", .{std.c.getErrno(err)});
                return Error.IoctlFailed;
            }

            ifreq.ifru.addr = ip.sa;
            std.c.ioctl(self.tun.handle, Net.SIOCSIFADDR, @ptrCast(c_void, &ifreq));

            ifreq.ifru.netmask = mask.sa;
            std.c.ioctl(self.tun.handle, Net.SIOCSIFNETMASK, @ptrCast(c_void, &ifreq));

            ifreq.ifru.mtu = 1500;
            std.c.ioctl(self.tun.handle, Net.SIOCSIFMTU, @ptrCast(c_void, &ifreq));

            std.c.ioctl(self.tun.handle, Net.SIOCGIFFLAGS, @ptrCast(c_void, &ifreq));
            ifreq.ifru.flags |= Net.IFF_UP | Net.IFF_RUNNING;
            ifreq.ifru.flags &= ~Net.IFF_MULTICAST;
            err = std.c.ioctl(self.tun.handle, Net.SIOCGIFFLAGS, @ptrCast(c_void, &ifreq));
            if(err < 0) {
                std.log.err("ioctl SIOCGIFFLAGS: {d}", .{std.c.getErrno(err)});
                return Error.IoctlFailed;
            }

            std.log.info("Driver: tun", .{});
            return self;
        }

        pub fn write(self: *Self, pkt: []u8) !void {
            try self.tun.write(pkt);
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
        driver: *Driver(T),
        local_ip: u32,
        
        pub fn init(alloc: *Allocator, conf: * const Config, user: T, handler: PacketHandler(T)) !Self {
            const ip = try Ip4Address.parse(conf.driver.local_addr, 0);
            std.log.info("Local IP: {s}", .{conf.driver.local_addr});
            return Self {
                .driver = try Driver(T).init(alloc, conf, user, handler),
                .local_ip = ip.sa.addr,
            };
        }

        pub fn write(self: *Self, src: u32, pkt: []u8) !void {
            const hdr = @ptrCast(*IpHeader, pkt);
            const payload_offset = hdr.ihl * 4;
            hdr.src = src;
            hdr.dst = self.local_ip;
            hdr.cksum = 0; // Zero before recalc
            hdr.cksum = cksum(pkt[0..payload_offset], 0);
            switch (hdr.proto) {
                6, 17 => { // TCP / UDP
                    var pseudo_hdr = PseudoHeader {
                        .src = src,
                        .dst = self.local_ip,
                        .proto = hdr.proto,
                        .len = std.mem.nativeToBig(u16, @intCast(u16, pkt.len) - payload_offset),
                    };
                    const pseudo_buf = @ptrCast(*[@sizeOf(PseudoHeader)/2]u16, @alignCast(2, &pseudo_hdr));
                    var pseudo_sum: u32 = 0;
                    for(pseudo_buf) |word| pseudo_sum += word;
                    const cksum_offset = payload_offset + if (hdr.proto == 6) @as(usize, 16) else 6;
                    const cksum_slice = std.mem.bytesAsSlice(u16, @alignCast(2, pkt[cksum_offset..2]));
                    std.mem.set(u16, cksum_slice, 0); // Zero before recalc
                    std.mem.set(u16, cksum_slice, cksum(pkt[payload_offset..], pseudo_sum));
                },
                else => {}, // No special handling
            }
            try self.driver.write(pkt);
        }

        fn cksum(buf: []u8, carry: u32) u16 {
            var sum: u32 = carry;
            const buf2 = std.mem.bytesAsSlice(u16, @alignCast(2, buf));
            for (buf2) |word| sum += word;
            if (buf.len % 2 != 0) {
                var last: u16 = 0;
                @ptrCast(*[2]u8, &last)[0] = buf[buf.len - 1];
                sum += last;
            }
            while (sum > 0xffff) {
                sum = (sum & 0xffff) + (sum >> 16);
            }
            return @intCast(u16, sum ^ 0xffff);
        }
    };
}
