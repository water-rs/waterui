//! The live preview host on the device: one instrumentation process the
//! CLI starts once and afterwards sends render requests to, so a warm run
//! pays no zygote fork, `System.load` relocation, GPU setup or font scan.
//!
//! The channel is an `adb forward` of the host's abstract-domain socket.
//! Each connection carries one render as single-line JSON frames: the host
//! greets first with the payload stamp it loaded, the CLI sends a
//! [`HostCommand`], and the host answers [`HostEvent::Rendered`] or
//! [`HostEvent::Failed`]. The greeting is the liveness and identity
//! signal, so a run learns from it whether the live host carries this
//! payload's stamp (reuse), carries a stale one or none (force-stop and
//! start a fresh host) or is absent (start one). A fresh host's readiness
//! is its own `logcat` line, logged once the libraries loaded and the
//! socket is bound; the run waits on it with `logcat -m 1`, then connects
//! once.
//!
//! A loaded library cannot be swapped, so a changed libraries stamp is the
//! one transition that must kill the host; the resources part is re-read
//! per render and never requires a restart.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use eyre::{Context as _, Result, bail};
use futures_util::{FutureExt as _, pin_mut, select};
use serde::{Deserialize, Serialize};
use smol::Timer;
use smol::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use smol::net::TcpStream;
use tracing::{debug, info};

use crate::android::adb::{Adb, recent_crash_log};
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
/// debug launcher and the socket bind — the backstop for a host that dies
/// without logging, not the start's expected duration.
const HOST_START_DEADLINE: Duration = Duration::from_secs(120);

/// The tag the host logs its start outcome under.
const HOST_LOG_TAG: &str = "HydrolysisPreview";

/// The start outcome the host logs once its socket is bound.
const HOST_SERVING: &str = "serving";

/// The start outcome the host logs when loading or binding failed.
const HOST_START_FAILED: &str = "failed to start";

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
            LineRead::Line(frame) => match parse_event(&frame)? {
                HostEvent::Rendered => Ok(()),
                HostEvent::Failed { error } => {
                    bail!("the preview host reported a render failure: {error}")
                }
                HostEvent::Hello { .. } => {
                    bail!("the preview host answered the render request with a greeting: {frame}")
                }
            },
        }
    }
}

/// The `adb forward` registration for the host socket — the local port a
/// run connects to.
#[derive(Debug)]
pub(super) struct HostForward {
    port: u16,
}

/// Ensure the adb server holds a forward from a local TCP port to the
/// host socket on `serial`, creating it when none is registered.
///
/// # Errors
/// Returns an error when the forward cannot be listed or created.
pub(super) async fn open_forward(host: &Host, adb: &Adb, serial: &str) -> Result<HostForward> {
    let remote = format!("localabstract:{HOST_SOCKET_NAME}");
    if let Some(forward) = adb
        .forwards(host, serial, FORWARD_DEADLINE)
        .await?
        .into_iter()
        .find(|forward| forward.remote == remote)
    {
        debug!(port = forward.local_port, "reusing the adb forward");
        return Ok(HostForward {
            port: forward.local_port,
        });
    }
    let port = adb.forward(host, serial, &remote, FORWARD_DEADLINE).await?;
    debug!(port, "created the adb forward");
    Ok(HostForward { port })
}

impl HostForward {
    /// A link to a host that loaded `stamp`'s libraries: the live host when
    /// its greeting names `stamp`, otherwise a fresh host started for
    /// `libraries` — once whatever answered with another stamp, or answered
    /// nothing, is force-stopped. Call it only after the payload push has
    /// landed: a fresh host loads what the device holds at its start.
    ///
    /// `since` is the run's `logcat -T` bound: the fresh host's readiness
    /// line must be newer than it.
    ///
    /// # Errors
    /// Returns an error when the probe's greeting is malformed or speaks
    /// another schema, when the force-stop or the `am instrument` start
    /// fails, when the fresh host reports a failed start or no readiness
    /// inside [`HOST_START_DEADLINE`], or when its greeting does not name
    /// `stamp`.
    pub(super) async fn link(
        &self,
        host: &Host,
        adb: &Adb,
        serial: &str,
        libraries: &[String],
        stamp: &str,
        since: &str,
    ) -> Result<HostLink> {
        match connect_and_greet(self.port).await? {
            Greet::Ready(link, held) if held == stamp => {
                info!(%stamp, "reusing the live preview host");
                return Ok(link);
            }
            Greet::Ready(_, held) => {
                info!(%held, %stamp, "restarting the preview host: the payload changed");
                force_stop_host(host, adb, serial).await?;
            }
            Greet::Silent => {
                info!(%stamp, "restarting the preview host: it accepts but sends no greeting");
                force_stop_host(host, adb, serial).await?;
            }
            Greet::Absent => info!(%stamp, "starting a preview host: none is running"),
        }
        start_host(host, adb, serial, libraries, stamp).await?;
        await_host_ready(host, adb, serial, stamp, since).await?;
        match connect_and_greet(self.port).await? {
            Greet::Ready(link, held) if held == stamp => Ok(link),
            Greet::Ready(_, held) => bail!(
                "the preview host started for stamp {stamp} reported ready, but the socket \
                 greeted with stamp {held}"
            ),
            Greet::Absent => bail!(
                "the preview host for stamp {stamp} reported ready, but the forwarded socket \
                 accepted no connection"
            ),
            Greet::Silent => bail!(
                "the preview host for stamp {stamp} reported ready, but sent no greeting within \
                 {} seconds",
                GREETING_DEADLINE.as_secs()
            ),
        }
    }
}

/// `am force-stop` the preview host package — the deliberate stop a
/// stamp change or a wedged host calls for, so the old process is gone
/// before the fresh one binds the socket name.
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
    let device_libraries = device_libraries.join(":");
    adb.shell_run(
        host,
        serial,
        &[
            "am",
            "instrument",
            "-e",
            "libraries",
            &device_libraries,
            "-e",
            "payloadStamp",
            stamp,
            PREVIEW_HOST_INSTRUMENTATION,
        ],
        AM_DEADLINE,
    )
    .await
    .wrap_err("failed to start the preview host")?;
    Ok(())
}

/// The line the host logs once its socket is bound — after the libraries
/// loaded — or once its start failed, for the host carrying `stamp`.
fn host_start_line(outcome: &str, stamp: &str) -> String {
    format!("preview host {outcome}: stamp {stamp}")
}

/// Block until the host started for `stamp` logs that it serves or that
/// its start failed. `logcat -m 1 -e` exits on the first matching line
/// newer than `since`, so the wait ends on the host's own signal; the
/// deadline only bounds a host that dies without logging either line.
async fn await_host_ready(
    host: &Host,
    adb: &Adb,
    serial: &str,
    stamp: &str,
    since: &str,
) -> Result<()> {
    let serving = host_start_line(HOST_SERVING, stamp);
    let failed = host_start_line(HOST_START_FAILED, stamp);
    let pattern = host_start_line(&format!("({HOST_SERVING}|{HOST_START_FAILED})"), stamp);
    let line = adb
        .first_log_line(
            host,
            serial,
            since,
            HOST_LOG_TAG,
            &pattern,
            HOST_START_DEADLINE,
        )
        .await;
    let line = match line {
        Ok(line) if line.contains(&serving) => {
            debug!(%stamp, "the preview host reported ready");
            return Ok(());
        }
        Ok(line) if line.contains(&failed) => format!("the preview host failed to start: {line}"),
        Ok(line) => format!("waiting for the preview host's start answered {line:?}"),
        Err(error) => format!("the preview host reported no start: {error}"),
    };
    let crash_log = recent_crash_log(host, adb, serial, Some(since)).await;
    bail!("{line}\n\n=== Crash Log ===\n{crash_log}")
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
        LineRead::Line(frame) => match parse_event(&frame)? {
            HostEvent::Hello { schema, stamp } if schema == HOST_PROTOCOL_SCHEMA => {
                Ok(Greet::Ready(link, stamp))
            }
            HostEvent::Hello { schema, .. } => bail!(
                "the preview host speaks protocol schema {schema}; this CLI speaks \
                 {HOST_PROTOCOL_SCHEMA} — update `water` or reinstall the preview host"
            ),
            HostEvent::Rendered | HostEvent::Failed { .. } => {
                bail!("the preview host greeted with {frame} where `hello` was expected")
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
