//! Polling IPv4/TCP/UDP stack backed by smoltcp and VirtIO Ethernet.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, AtomicUsize, Ordering};

use kor_virtio::NetworkDevice;
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::{tcp, udp};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpAddress, IpCidr, IpEndpoint, Ipv4Address};
use spin::Mutex;

const GUEST_IP: [u8; 4] = [10, 0, 2, 15];
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];
/// QEMU user-network DNS resolver address (wired up when DNS support lands).
#[allow(dead_code)]
const DNS_IP: [u8; 4] = [10, 0, 2, 3];
const ETHERNET_MTU: usize = 1514;
const RX_BUFFER_SIZE: usize = 64 * 1024;
const TX_BUFFER_SIZE: usize = 64 * 1024;
const UDP_PAYLOAD_SIZE: usize = 4096;
const UDP_PACKET_COUNT: usize = 8;
const CONNECT_TIMEOUT_SECONDS: u64 = 10;

static DEVICE: Mutex<Option<Arc<dyn NetworkDevice>>> = Mutex::new(None);
static STACK: Mutex<Option<NetworkStack>> = Mutex::new(None);
static NEXT_SOCKET: AtomicUsize = AtomicUsize::new(1);
static NEXT_PORT: AtomicU16 = AtomicU16::new(49152);

#[derive(Clone, Copy)]
enum SocketKind {
    Tcp,
    Udp,
}

struct SocketState {
    kind: SocketKind,
    handle: SocketHandle,
    local_port: u16,
    remote: Option<([u8; 4], u16)>,
}

struct NetworkStack {
    interface: Interface,
    sockets: SocketSet<'static>,
    states: BTreeMap<usize, SocketState>,
    received: Option<Vec<u8>>,
}

struct DeviceAdapter {
    device: Arc<dyn NetworkDevice>,
    received: Option<Vec<u8>>,
}

struct AdapterRxToken(Vec<u8>);
struct AdapterTxToken(Arc<dyn NetworkDevice>);

impl RxToken for AdapterRxToken {
    fn consume<R, F>(self, callback: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        callback(&self.0)
    }
}

impl TxToken for AdapterTxToken {
    fn consume<R, F>(self, length: usize, callback: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut frame = vec![0; length];
        let result = callback(&mut frame);
        let _ = self.0.send(&frame);
        result
    }
}

impl DeviceAdapter {
    fn new(device: Arc<dyn NetworkDevice>, received: Option<Vec<u8>>) -> Self {
        Self { device, received }
    }

    fn receive_frame(&mut self) -> bool {
        if self.received.is_some() {
            return true;
        }
        let mut frame = vec![0; ETHERNET_MTU];
        let Some(length) = self.device.receive(&mut frame) else {
            return false;
        };
        frame.truncate(length);
        self.received = Some(frame);
        true
    }
}

impl Device for DeviceAdapter {
    type RxToken<'a> = AdapterRxToken;
    type TxToken<'a> = AdapterTxToken;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.receive_frame().then(|| {
            let frame = self.received.take().unwrap();
            (AdapterRxToken(frame), AdapterTxToken(self.device.clone()))
        })
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        Some(AdapterTxToken(self.device.clone()))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.medium = Medium::Ethernet;
        capabilities.max_transmission_unit = ETHERNET_MTU;
        capabilities.max_burst_size = None;
        capabilities
    }
}

fn timestamp() -> Instant {
    let milliseconds = kor_arch::now().as_millis() as i64;
    Instant::from_millis(milliseconds as i64)
}

fn poll_locked(stack: &mut NetworkStack) {
    let Some(device) = DEVICE.lock().clone() else {
        return;
    };
    loop {
        let mut adapter = DeviceAdapter::new(device.clone(), stack.received.take());
        let result = stack
            .interface
            .poll(timestamp(), &mut adapter, &mut stack.sockets);
        stack.received = adapter.received;
        if matches!(result, smoltcp::iface::PollResult::None) {
            break;
        }
    }
}

/// Install and configure the polling network device.
pub fn install(device: Arc<dyn NetworkDevice>) {
    let mut adapter = DeviceAdapter::new(device.clone(), None);
    let mut config = Config::new(HardwareAddress::Ethernet(EthernetAddress(
        device.mac_address(),
    )));
    config.random_seed = 0x4b4f_5249_5800_0001;
    let mut interface = Interface::new(config, &mut adapter, timestamp());
    interface.update_ip_addrs(|addresses| {
        addresses
            .push(IpCidr::new(
                IpAddress::v4(GUEST_IP[0], GUEST_IP[1], GUEST_IP[2], GUEST_IP[3]),
                24,
            ))
            .unwrap();
    });
    interface
        .routes_mut()
        .add_default_ipv4_route(Ipv4Address::new(
            GATEWAY_IP[0],
            GATEWAY_IP[1],
            GATEWAY_IP[2],
            GATEWAY_IP[3],
        ))
        .unwrap();
    *DEVICE.lock() = Some(device);
    *STACK.lock() = Some(NetworkStack {
        interface,
        sockets: SocketSet::new(vec![]),
        states: BTreeMap::new(),
        received: adapter.received,
    });
}

/// Create an IPv4 stream or datagram socket and return its internal id.
pub fn socket(kind: i32) -> Result<usize, isize> {
    let mut guard = STACK.lock();
    let stack = guard.as_mut().ok_or(-100isize)?;
    let kind = match kind {
        1 => SocketKind::Tcp,
        2 => SocketKind::Udp,
        _ => return Err(-94),
    };
    let handle = match kind {
        SocketKind::Tcp => stack.sockets.add(tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; RX_BUFFER_SIZE]),
            tcp::SocketBuffer::new(vec![0; TX_BUFFER_SIZE]),
        )),
        SocketKind::Udp => stack.sockets.add(udp::Socket::new(
            udp::PacketBuffer::new(
                vec![udp::PacketMetadata::EMPTY; UDP_PACKET_COUNT],
                vec![0; UDP_PAYLOAD_SIZE * UDP_PACKET_COUNT],
            ),
            udp::PacketBuffer::new(
                vec![udp::PacketMetadata::EMPTY; UDP_PACKET_COUNT],
                vec![0; UDP_PAYLOAD_SIZE * UDP_PACKET_COUNT],
            ),
        )),
    };
    let id = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
    let local_port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
    if let SocketKind::Udp = kind {
        stack
            .sockets
            .get_mut::<udp::Socket>(handle)
            .bind(local_port)
            .map_err(|_| -98isize)?;
    }
    stack.states.insert(
        id,
        SocketState {
            kind,
            handle,
            local_port,
            remote: None,
        },
    );
    Ok(id)
}

/// Close and remove a socket.
pub fn close(id: usize) -> bool {
    let mut guard = STACK.lock();
    let Some(stack) = guard.as_mut() else {
        return false;
    };
    let Some(state) = stack.states.remove(&id) else {
        return false;
    };
    stack.sockets.remove(state.handle);
    true
}

/// Connect a socket to a remote IPv4 endpoint.
pub fn connect(id: usize, ip: [u8; 4], port: u16) -> Result<(), isize> {
    let mut guard = STACK.lock();
    let stack = guard.as_mut().ok_or(-100isize)?;
    let (kind, handle, local_port) = {
        let state = stack.states.get_mut(&id).ok_or(-9isize)?;
        state.remote = Some((ip, port));
        (state.kind, state.handle, state.local_port)
    };
    match kind {
        SocketKind::Udp => return Ok(()),
        SocketKind::Tcp => {
            let endpoint = IpEndpoint::new(IpAddress::v4(ip[0], ip[1], ip[2], ip[3]), port);
            stack
                .sockets
                .get_mut::<tcp::Socket>(handle)
                .connect(stack.interface.context(), endpoint, local_port)
                .map_err(|_| -111isize)?;
        }
    }
    let deadline = timestamp().total_millis() as u64 + CONNECT_TIMEOUT_SECONDS * 1000;
    loop {
        poll_locked(stack);
        let socket = stack.sockets.get::<tcp::Socket>(handle);
        if socket.may_send() {
            return Ok(());
        }
        if !socket.is_open() || timestamp().total_millis() as u64 >= deadline {
            return Err(-110);
        }
        core::hint::spin_loop();
    }
}

/// Send bytes through a connected socket or to an explicit destination.
pub fn send(id: usize, data: &[u8], destination: Option<([u8; 4], u16)>) -> Result<usize, isize> {
    let mut guard = STACK.lock();
    let stack = guard.as_mut().ok_or(-100isize)?;
    let state = stack.states.get_mut(&id).ok_or(-9isize)?;
    match state.kind {
        SocketKind::Tcp => {
            let socket = stack.sockets.get_mut::<tcp::Socket>(state.handle);
            if !socket.may_send() {
                return Err(-32);
            }
            let count = socket.send_slice(data).map_err(|_| -11isize)?;
            count
        }
        SocketKind::Udp => {
            let (ip, port) = destination.or(state.remote).ok_or(-89isize)?;
            state.remote = Some((ip, port));
            let endpoint = IpEndpoint::new(IpAddress::v4(ip[0], ip[1], ip[2], ip[3]), port);
            stack
                .sockets
                .get_mut::<udp::Socket>(state.handle)
                .send_slice(data, endpoint)
                .map_err(|_| -11isize)?;
            data.len()
        }
    }
    .pipe(|count| {
        poll_locked(stack);
        Ok(count)
    })
}

trait Pipe: Sized {
    fn pipe<R>(self, callback: impl FnOnce(Self) -> R) -> R {
        callback(self)
    }
}
impl<T> Pipe for T {}

/// Receive bytes and return their source address.
pub fn recv(id: usize, data: &mut [u8]) -> Result<(usize, [u8; 4], u16), isize> {
    loop {
        let mut guard = STACK.lock();
        let stack = guard.as_mut().ok_or(-100isize)?;
        poll_locked(stack);
        let state = stack.states.get(&id).ok_or(-9isize)?;
        match state.kind {
            SocketKind::Tcp => {
                let socket = stack.sockets.get_mut::<tcp::Socket>(state.handle);
                if socket.can_recv() {
                    let count = socket.recv_slice(data).map_err(|_| -5isize)?;
                    let (ip, port) = state.remote.unwrap_or(([0; 4], 0));
                    return Ok((count, ip, port));
                }
                if !socket.may_recv() {
                    let (ip, port) = state.remote.unwrap_or(([0; 4], 0));
                    return Ok((0, ip, port));
                }
            }
            SocketKind::Udp => {
                let socket = stack.sockets.get_mut::<udp::Socket>(state.handle);
                if socket.can_recv() {
                    let (packet, endpoint) = socket.recv().map_err(|_| -5isize)?;
                    let count = data.len().min(packet.len());
                    data[..count].copy_from_slice(&packet[..count]);
                    let ip = match endpoint.endpoint.addr {
                        IpAddress::Ipv4(address) => address.octets(),
                    };
                    return Ok((count, ip, endpoint.endpoint.port));
                }
            }
        }
        drop(guard);
        wait();
    }
}

/// Poll the device and report whether a socket can be read without blocking.
pub fn has_data(id: usize) -> bool {
    let mut guard = STACK.lock();
    let Some(stack) = guard.as_mut() else {
        return false;
    };
    poll_locked(stack);
    let Some(state) = stack.states.get(&id) else {
        return false;
    };
    match state.kind {
        SocketKind::Tcp => {
            let socket = stack.sockets.get::<tcp::Socket>(state.handle);
            socket.can_recv() || !socket.may_recv()
        }
        SocketKind::Udp => stack.sockets.get::<udp::Socket>(state.handle).can_recv(),
    }
}

/// Poll the network interface once.
pub fn poll() {
    let mut guard = STACK.lock();
    if let Some(stack) = guard.as_mut() {
        poll_locked(stack);
    }
}

/// Poll the network stack and let the CPU service pending device work.
pub fn wait() {
    poll();
    core::hint::spin_loop();
}

/// Return the QEMU user-network DNS server address.
#[expect(dead_code, reason = "staged: pending wiring")]
pub fn dns_server() -> [u8; 4] {
    DNS_IP
}
