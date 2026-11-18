//! Which herdr servers a machine has: the herdr TUI's own server (`ping`)
//! and herdr-core (`hello`, or installed so that `herdr-core connect` would
//! start it).

use std::sync::Arc;
use std::time::Duration;

use crate::error::Error;
use crate::hc::{CoreClient, find_core_binary};
use crate::herdr::HerdrClient;
use crate::profile::Backend;
use crate::transport::Transport;

/// Upper bound for one probe. A server that takes longer is counted as not
/// there; probes never hold up a connection.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a server is on the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// It answers now.
    Running,
    /// herdr-core is installed but not running: only a profile set to
    /// herdr-core starts it (`herdr-core connect`); `auto` never does.
    Installed,
    /// Surely not there.
    Absent,
}

/// A server found on a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detected {
    pub backend: Backend,
    /// It answers now; otherwise it is installed and would be started by a
    /// profile set to it ([`Presence::Installed`]).
    pub running: bool,
    /// The probe gave no clear answer (a timeout, a broken connection, a
    /// failed lookup): the server may or may not be there. `running` is then
    /// false. A form must not save on such a result.
    pub unclear: bool,
}

/// The servers a machine has, TUI first. Both are asked at the same time,
/// each within [`PROBE_TIMEOUT`]; a backend whose probe gave no clear answer
/// is listed with `unclear` set, one that is surely absent is left out, so an
/// empty list means neither runs nor is installed. Nothing is started. A
/// server that is only installed can only be reached by a profile set to it:
/// a form must ask before saving such a profile as herdr-core.
pub async fn detect_backends(
    transport: &Arc<Transport>,
    session: Option<&str>,
    socket_override: Option<&str>,
) -> Vec<Detected> {
    let (tui, core) = tokio::join!(
        probe(transport, Backend::Tui, session, socket_override),
        probe(transport, Backend::Core, session, socket_override),
    );
    summarize(tui, core)
}

/// The list [`detect_backends`] returns for the two probe results.
fn summarize(tui: Option<Presence>, core: Option<Presence>) -> Vec<Detected> {
    [(tui, Backend::Tui), (core, Backend::Core)]
        .into_iter()
        .filter_map(|(found, backend)| match found {
            Some(Presence::Running) => Some(Detected {
                backend,
                running: true,
                unclear: false,
            }),
            Some(Presence::Installed) => Some(Detected {
                backend,
                running: false,
                unclear: false,
            }),
            Some(Presence::Absent) => None,
            None => Some(Detected {
                backend,
                running: false,
                unclear: true,
            }),
        })
        .collect()
}

/// Whether `backend` is on the machine, within [`PROBE_TIMEOUT`]; `None` when
/// that is not clear (a timeout, a connection that broke, a lookup that
/// failed). herdr-core whose socket does not answer counts as
/// [`Presence::Installed`] when its binary is there. Nothing is started.
pub async fn probe(
    transport: &Arc<Transport>,
    backend: Backend,
    session: Option<&str>,
    socket_override: Option<&str>,
) -> Option<Presence> {
    let check = async {
        let connected = match backend {
            Backend::Tui => HerdrClient::connect(transport.clone(), socket_override, session)
                .await
                .map(drop),
            Backend::Core => {
                match CoreClient::connect(transport.clone(), socket_override, session, false).await
                {
                    // A core in the wrong generation is still a core here.
                    Ok(_) | Err(Error::Incompatible { .. }) => Ok(()),
                    Err(e) => Err(e),
                }
            }
            Backend::Auto => return None,
        };
        let error = match connected {
            Ok(()) => return Some(Presence::Running),
            Err(e) => e,
        };
        tracing::debug!(backend = backend.as_str(), error = %error, "probe: not running");
        // An error from a dying connection says nothing about the server.
        if !error.is_absent() || transport.is_closed() {
            return None;
        }
        if backend != Backend::Core || socket_override.is_some() {
            return Some(Presence::Absent);
        }
        match find_core_binary(transport).await {
            Ok(Some(_)) => Some(Presence::Installed),
            Ok(None) if !transport.is_closed() => Some(Presence::Absent),
            // The lookup failed (or the connection went meanwhile): unclear.
            _ => None,
        }
    };
    match tokio::time::timeout(PROBE_TIMEOUT, check).await {
        Ok(found) => found,
        Err(_) => {
            tracing::debug!(backend = backend.as_str(), "probe: timed out");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::LocalTransport;

    fn local() -> Arc<Transport> {
        Arc::new(Transport::Local(LocalTransport))
    }

    #[test]
    fn unclear_probes_stay_in_the_list() {
        let found = summarize(Some(Presence::Running), None);
        assert_eq!(found.len(), 2);
        assert!(found[0].running && !found[0].unclear);
        assert_eq!(found[1].backend, Backend::Core);
        assert!(found[1].unclear && !found[1].running);

        let found = summarize(None, None);
        assert!(
            found.iter().all(|d| d.unclear),
            "both unclear: not \"nothing found\""
        );

        let found = summarize(Some(Presence::Absent), Some(Presence::Installed));
        assert_eq!(found.len(), 1);
        assert!(!found[0].running && !found[0].unclear);

        assert!(summarize(Some(Presence::Absent), Some(Presence::Absent)).is_empty());
    }

    #[tokio::test]
    async fn a_missing_socket_is_a_clear_no() {
        let socket = std::env::temp_dir().join(format!("mc-test-{}-none.sock", std::process::id()));
        let found = probe(&local(), Backend::Tui, None, socket.to_str()).await;
        assert_eq!(found, Some(Presence::Absent));
    }

    #[tokio::test]
    async fn a_server_that_does_not_answer_is_no_answer() {
        let dir = std::env::temp_dir().join(format!("mc-test-{}-mute", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let socket = dir.join("herdr.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        // Accepts and keeps the connection open, never answering.
        let held = tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                open.push(stream);
            }
        });
        let found = probe(&local(), Backend::Tui, None, socket.to_str()).await;
        assert_eq!(found, None, "not clear: neither found nor absent");
        held.abort();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
