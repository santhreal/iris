//! An X display that forwards each connection to a real X server and
//! records, in order, when it accepts a connection, when the server
//! accepts a connection's setup, and when the connection's client closes
//! it. It passes the server's answer to the first connection's setup on
//! late, as a loaded server answers. File descriptors a client sends with
//! a request are not forwarded.

use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::linux::net::SocketAddrExt as _;
use std::os::unix::net::{SocketAddr, UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What happened to a connection. Connections are numbered in the order
/// the proxy accepted them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// The proxy accepted the connection from its client.
    Opened(usize),
    /// The server answered the connection's setup with Success.
    SetUp(usize),
    /// The connection's client closed it.
    Closed(usize),
}

/// A display forwarding to another. Its threads run until the test
/// process exits.
pub struct Proxy {
    /// The display name clients connect to.
    pub display: String,
    events: Arc<Mutex<Vec<Event>>>,
}

impl Proxy {
    /// A proxy on a display number no server uses, in front of `server`,
    /// an X display name `:N`, that passes the server's answer to the
    /// first connection's setup on `hold` after the server sends it. An X
    /// client on Linux connects to display `:N` through the abstract
    /// socket `/tmp/.X11-unix/XN` first.
    pub fn new(server: &str, hold: Duration) -> Proxy {
        let upstream = format!("/tmp/.X11-unix/X{}", &server[1..]);
        for n in 400..600 {
            let name = format!("/tmp/.X11-unix/X{n}");
            // A server listening on the path would answer a client that
            // found no abstract socket.
            if Path::new(&name).exists() {
                continue;
            }
            let address = SocketAddr::from_abstract_name(name.as_bytes()).unwrap();
            let Ok(listener) = UnixListener::bind_addr(&address) else {
                continue;
            };
            let events = Arc::new(Mutex::new(Vec::new()));
            let log = events.clone();
            std::thread::spawn(move || {
                let address = SocketAddr::from_abstract_name(upstream.as_bytes()).unwrap();
                for (id, client) in listener.incoming().enumerate() {
                    log.lock().unwrap().push(Event::Opened(id));
                    let (Ok(client), Ok(server)) = (client, UnixStream::connect_addr(&address))
                    else {
                        continue;
                    };
                    let hold = if id == 0 { hold } else { Duration::ZERO };
                    forward(id, client, server, hold, &log);
                }
            });
            return Proxy {
                display: format!(":{n}"),
                events,
            };
        }
        panic!("every X display number from :400 to :599 is in use");
    }

    /// The events so far, in order.
    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }
}

/// Copy bytes both ways between `client` and `server` until either end
/// closes, and record connection `id`'s setup and close. The server's
/// first answer reaches the client `hold` after the server sends it. Each
/// event is recorded before the other end reads of it.
fn forward(
    id: usize,
    client: UnixStream,
    server: UnixStream,
    hold: Duration,
    events: &Arc<Mutex<Vec<Event>>>,
) {
    let (mut requests, mut to_server) = (client.try_clone().unwrap(), server.try_clone().unwrap());
    let log = events.clone();
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut requests, &mut to_server);
        log.lock().unwrap().push(Event::Closed(id));
        let _ = to_server.shutdown(Shutdown::Both);
    });
    let log = events.clone();
    std::thread::spawn(move || {
        let (mut replies, mut to_client) = (server, client);
        let mut buf = [0; 16 * 1024];
        let mut first = true;
        while let Ok(len @ 1..) = replies.read(&mut buf) {
            // The first byte of the server's first answer is the setup's
            // status, 1 for Success.
            if std::mem::take(&mut first) {
                std::thread::sleep(hold);
                if buf[0] == 1 {
                    log.lock().unwrap().push(Event::SetUp(id));
                }
            }
            if to_client.write_all(&buf[..len]).is_err() {
                break;
            }
        }
        let _ = to_client.shutdown(Shutdown::Both);
    });
}
