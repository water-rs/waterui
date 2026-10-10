//! The live preview host on the device: one instrumentation process the
//! CLI starts once and afterwards sends render requests to, so a warm run
//! pays no zygote fork, `System.load` relocation, GPU setup or font scan.
//!
//! The channel is an `adb forward` of the host's abstract-domain socket.
//! Each connection carries one render as single-line JSON frames: the host
//! greets first with the payload stamp it loaded, the CLI sends a
//! [`HostCommand`], and the host answers [`HostEvent::Rendered`] or
//! [`HostEvent::Failed`]. The greeting is the liveness and readiness
//! signal — the socket exists only once the libraries are loaded — so a
//! run learns from it whether the live host carries this payload's stamp
//! (reuse), carries a stale one (force-stop and start a fresh host) or is
//! absent (start one).
//!
//! A loaded library cannot be swapped, so a changed libraries stamp is the
//! one transition that must kill the host; the resources part is re-read
//! per render and never requires a restart.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use eyre::{Context as _, Result, bail};
use futures_util::{FutureExt as _, pin_mut, select};
use serde::{Deserialize, Serialize};
use smol::Timer;
use smol::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use smol::net::TcpStream;
use tracing::{debug, info};

use crate::android::adb::Adb;
use crate::hydrolysis::android::{PREVIEW_HOST_INSTRUMENTATION, PREVIEW_HOST_PACKAGE};
use crate::toolchain::Host;

/// The wire schema this CLI speaks; the host reports its own in the
/// greeting and a mismatch is an error, not a dialect to negotiate.
const HOST_PROTOCOL_SCHEMA: u32 = 1;

/// The abstract-domain socket the live host listens on — `localabstract:`
/// on the `adb forward` side, `LocalServerSocket` on the device side.
const HOST_SOCKET_NAME: &str = "dev.waterui.hydrolysis.preview";

/// `forward --list`/`forward` round trips.
const FORWARD_DEADLINE: Duration = Duration::from_secs(30);

/// One probe's connect and its greeting read.
const GREETING_DEADLINE: Duration = Duration::from_secs(10);

/// The bound on a host start: the process fork, its `System.load` of the
/// debug launcher and the socket bind.
const HOST_START_DEADLINE: Duration = Duration::from_secs(120);

/// The spacing between a refused or wrong-answer and the next connect —
/// the wait for the *new* host to bind the socket, not a readiness delay:
/// readiness is the greeting the host itself sends.
const CONNECT_DELAY: Duration = Duration::from_millis(50);

/// The reply bound for a render request — the same wall the instrumentation
/// `-w` wait carried.
const RENDER_DEADLINE: Duration = Duration::from_mins(3);

/// `am instrument` and `am force-stop` round trips.
const AM_DEADLINE: Duration = Duration::from_secs(30);

/// A request the CLI sends the live host, serialized as one JSON line.
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostCommand<'a> {
    /// Render once with the run config and assets root at these paths,
    /// relative to the host's `filesDir` exactly as the instrumentation
    /// extras carried them.
    Render {
        run_config: &'a str,
        assets_root: &'a str,
    },
}

/// A frame the live host sends, parsed from one JSON line.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum HostEvent {
    /// The greeting every connection opens with: the protocol schema and
    /// the payload stamp the host loaded.
    Hello { schema: u32, stamp: String },
    /// The render completed; its output sits where the run config put it.
    Rendered,
    /// The render failed; `error` is the host's report.
    Failed { error: String },
}

/// A live connection to the preview host — the greeting already exchanged.
#[derive(Debug)]
pub(super) struct HostLink {
    stream: BufReader<TcpStream>,
}

impl HostLink {
    /// Send the render request and await its reply.
    ///
    /// # Errors
    /// Returns an error when the write or the reply read fails, the reply
    /// names a failure, is not a frame, or is a frame that is not a render
    /// answer — the message always names what arrived.
    pub(super) async fn render(&mut self, run_config: &str, assets_root: &str) -> Result<()> {
        let line = serde_json::to_string(&HostCommand::Render {
            run_config,
            assets_root,
        })
        .wrap_err("failed to serialize the render request")?;
        let send = async {
            self.stream.write_all(line.as_bytes()).await?;
            self.stream.write_all(b"\n").await?;
            self.stream.flush().await
        };
        send.await
            .wrap_err("failed to send the render request to the preview host")?;
        match read_line(&mut self.stream, RENDER_DEADLINE).await? {
            LineRead::Closed => {
                bail!("the preview host closed the connection without answering the render request")
            }
            LineRead::TimedOut => bail!(
                "the preview host did not answer the render request within {} seconds",
                RENDER_DEADLINE.as_secs()
            ),
            LineRead::Line(line) => match parse_event(&line)? {
                HostEvent::Rendered => Ok(()),
                HostEvent::Failed { error } => {
                    bail!("the preview host reported a render failure: {error}")
                }
                other => {
                    bail!("the preview host answered the render request with {other:?}: {line}")
                }
            },
        }
    }
}

/// What the probe of the forwarded port found.
#[derive(Debug)]
enum Probe {
    /// A host greeted with this run's payload stamp — reuse it.
    Live(HostLink),
    /// Nothing answered: no listener, or a forward whose far end is gone.
    Absent,
    /// A host greeted with a different stamp — its loaded payload is stale.
    Stale(String),
    /// A socket accepted the connection but sent no greeting in time —
    /// something is there and wedged.
    Silent,
}

/// The outcome of [`check_for_live_host`]: the forward's local port and
/// what the probe learned. A run probes beside its payload push and turns
/// the probe into a [`HostLink`] once the push lands — a fresh host must
/// start only after the payload it will report as loaded is in place.
#[derive(Debug)]
pub(super) struct HostCheck {
    port: u16,
    probe: Probe,
}

/// Ensure the `adb forward` for the host socket exists, then probe it for
/// a live host carrying `stamp`.
///
/// # Errors
/// Returns an error when the forward cannot be listed or created, or the
/// host's greeting is malformed or speaks another schema.
pub(super) async fn check_for_live_host(
    host: &Host,
    adb: &Adb,
    serial: &str,
    stamp: &str,
) -> Result<HostCheck> {
    let port = ensure_forward(host, adb, serial).await?;
    let probe = match connect_and_greet(port).await? {
        Greet::Ready(link, held) if held == stamp => {
            debug!(%held, "the live preview host carries this payload's stamp");
            Probe::Live(link)
        }
        Greet::Ready(_, held) => {
            debug!(%held, expected = %stamp, "the live preview host carries a stale payload");
            Probe::Stale(held)
        }
        Greet::Absent => Probe::Absent,
        Greet::Silent => Probe::Silent,
    };
    Ok(HostCheck { port, probe })
}

impl HostCheck {
    /// Turn the probe into a link: reuse a live host, or — once the caller's
    /// payload push has landed — stop whatever answered and start a fresh
    /// host for `libraries`.
    ///
    /// # Errors
    /// Returns an error when the force-stop, the `am instrument` start, or
    /// the wait for the new host's greeting fails.
    pub(super) async fn into_link(
        self,
        host: &Host,
        adb: &Adb,
        serial: &str,
        libraries: &[String],
        stamp: &str,
    ) -> Result<HostLink> {
        match self.probe {
            Probe::Live(link) => {
                info!("reusing the live preview host");
                Ok(link)
            }
            probe => {
                // `am instrument` itself force-stops the package, but it
                // returns before the kill completes: stop deliberately so
                // the dying host's socket cannot answer the wait below.
                let stop = match &probe {
                    Probe::Absent => {
                        info!("starting a preview host: none is running");
                        false
                    }
                    Probe::Stale(held) => {
                        info!(%held, "restarting the preview host: the payload changed");
                        true
                    }
                    Probe::Silent => {
                        info!("restarting the preview host: it accepts but answers nothing");
                        true
                    }
                    Probe::Live(_) => unreachable!("handled above"),
                };
                if stop {
                    force_stop_host(host, adb, serial).await?;
                }
                start_host(host, adb, serial, libraries, stamp).await?;
                await_host(self.port, stamp).await
            }
        }
    }
}

/// The `adb forward` local port for the host socket, creating the
/// registration when the adb server does not already hold one.
async fn ensure_forward(host: &Host, adb: &Adb, serial: &str) -> Result<u16> {
    let remote = format!("localabstract:{HOST_SOCKET_NAME}");
    if let Some(forward) = adb
        .forwards(host, serial, FORWARD_DEADLINE)
        .await?
        .into_iter()
        .find(|forward| forward.remote == remote)
    {
        debug!(port = forward.local_port, "reusing the adb forward");
        return Ok(forward.local_port);
    }
    let port = adb.forward(host, serial, &remote, FORWARD_DEADLINE).await?;
    debug!(port, "created the adb forward");
    Ok(port)
}

/// `am force-stop` the preview host package — the deliberate stop a
/// stamp change or a wedged host calls for.
async fn force_stop_host(host: &Host, adb: &Adb, serial: &str) -> Result<()> {
    adb.shell_run(
        host,
        serial,
        &["am", "force-stop", PREVIEW_HOST_PACKAGE],
        AM_DEADLINE,
    )
    .await
    .wrap_err("failed to force-stop the preview host")?;
    Ok(())
}

/// Start the preview host: `am instrument` without `-w` returns once the
/// instrumentation is launched, and the process it spawns serves until the
/// package is force-stopped or replaced. The `payloadStamp` extra is what
/// the host reports in its greeting — its identity, not a value it can
/// discover itself.
async fn start_host(
    host: &Host,
    adb: &Adb,
    serial: &str,
    libraries: &[String],
    stamp: &str,
) -> Result<()> {
    let device_libraries: Vec<String> = libraries
        .iter()
        .map(|path| format!("{}/{path}", super::FILES_PREVIEW_DIR))
        .collect();
    let words = [
        "am".to_string(),
        "instrument".to_string(),
        "-e".to_string(),
        "libraries".to_string(),
        device_libraries.join(":"),
        "-e".to_string(),
        "payloadStamp".to_string(),
        stamp.to_string(),
        PREVIEW_HOST_INSTRUMENTATION.to_string(),
    ];
    adb.shell_run(
        host,
        serial,
        &words.iter().map(String::as_str).collect::<Vec<_>>(),
        AM_DEADLINE,
    )
    .await
    .wrap_err("failed to start the preview host")?;
    Ok(())
}

/// Wait for the freshly started host to bind the socket and greet with
/// `stamp`. Every connect is itself the readiness probe — a refused
/// connection or a wrong answer is "not up yet", never an assumed delay —
/// so the loop's only blind element is the final deadline, which names
/// whatever answered last.
async fn await_host(port: u16, stamp: &str) -> Result<HostLink> {
    let deadline = Instant::now() + HOST_START_DEADLINE;
    let mut last_seen = "nothing answered".to_string();
    loop {
        match connect_and_greet(port).await? {
            Greet::Ready(link, held) if held == stamp => return Ok(link),
            Greet::Ready(_, held) => {
                // The host this run is replacing can still own the socket
                // while its force-stop lands; its stamp is the stale one.
                last_seen = format!("a host carrying stamp {held}");
            }
            Greet::Absent => {}
            Greet::Silent => {
                last_seen = "a socket that accepted but sent no greeting".to_string();
            }
        }
        if Instant::now() >= deadline {
            bail!(
                "the preview host did not come up within {} seconds — last seen: {last_seen}",
                HOST_START_DEADLINE.as_secs()
            );
        }
        Timer::after(CONNECT_DELAY).await;
    }
}

/// What connecting and reading the greeting produced.
enum Greet {
    /// A greeting arrived, naming the stamp the host loaded.
    Ready(HostLink, String),
    /// Nothing usable answered — refused, or closed without a greeting.
    Absent,
    /// A socket accepted but sent no greeting inside the deadline.
    Silent,
}

/// Connect to the forward's local port and read the host's greeting.
/// Connect failures and a closed-before-greeting connection both mean
/// "no live host"; a silent-but-accepted socket means "wedged host".
async fn connect_and_greet(port: u16) -> Result<Greet> {
    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
    let connect = TcpStream::connect(addr).fuse();
    let timeout = Timer::after(GREETING_DEADLINE).fuse();
    pin_mut!(connect, timeout);
    let stream = select! {
        stream = connect => match stream {
            Ok(stream) => stream,
            Err(error) => {
                debug!(%error, "the preview host forward refused the connect");
                return Ok(Greet::Absent);
            }
        },
        _ = timeout => return Ok(Greet::Absent),
    };
    let mut link = HostLink {
        stream: BufReader::new(stream),
    };
    match read_line(&mut link.stream, GREETING_DEADLINE).await? {
        LineRead::Closed => Ok(Greet::Absent),
        LineRead::TimedOut => Ok(Greet::Silent),
        LineRead::Line(line) => match parse_event(&line)? {
            HostEvent::Hello { schema, stamp } if schema == HOST_PROTOCOL_SCHEMA => {
                Ok(Greet::Ready(link, stamp))
            }
            HostEvent::Hello { schema, .. } => bail!(
                "the preview host speaks protocol schema {schema}; this CLI speaks \
                 {HOST_PROTOCOL_SCHEMA} — update `water` or reinstall the preview host"
            ),
            other => {
                bail!("the preview host greeted with {other:?} where `hello` was expected: {line}")
            }
        },
    }
}

/// One frame's read inside a deadline.
enum LineRead {
    /// A full or partial line arrived.
    Line(String),
    /// The peer closed before any byte of a next frame.
    Closed,
    /// The deadline elapsed first.
    TimedOut,
}

/// Read one newline-terminated frame off `stream`, bounded by `deadline`.
/// A partial line at EOF still arrives as a [`LineRead::Line`], so its
/// parse failure can name it.
async fn read_line(stream: &mut BufReader<TcpStream>, deadline: Duration) -> Result<LineRead> {
    let mut bytes = Vec::new();
    let read = stream.read_until(b'\n', &mut bytes).fuse();
    let timeout = Timer::after(deadline).fuse();
    pin_mut!(read, timeout);
    match select! {
        result = read => result,
        _ = timeout => return Ok(LineRead::TimedOut),
    } {
        Ok(0) => Ok(LineRead::Closed),
        Ok(_) => {
            if bytes.last() == Some(&b'\n') {
                bytes.pop();
            }
            Ok(LineRead::Line(String::from_utf8_lossy(&bytes).into_owned()))
        }
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            Ok(LineRead::Line(String::from_utf8_lossy(&bytes).into_owned()))
        }
        Err(error) => Err(error).wrap_err("failed to read the preview host's reply"),
    }
}

/// Parse one frame into its [`HostEvent`], naming the line when it is not
/// one.
fn parse_event(line: &str) -> Result<HostEvent> {
    serde_json::from_str(line)
        .wrap_err_with(|| format!("the preview host sent a malformed frame: {line}"))
}
