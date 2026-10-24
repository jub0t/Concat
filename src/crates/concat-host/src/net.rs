// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The one way the host reaches the network: models, sounds and update
//! checks all go through [`agent`], and the manifests that vouch for a
//! download through [`strict_agent`].
//!
//! An HTTPS connection is trusted when its certificate chains to a root
//! Concat ships (Mozilla's list, as webpki-roots carries it) or to a root
//! the system trusts. ureq trusted the shipped list alone, so a network that
//! re-signs HTTPS with a root of its own - a proxy or a GitHub accelerator
//! in China, an antivirus that scans HTTPS, a company's gateway - failed
//! every download with "invalid peer certificate: UnknownIssuer", while
//! the browser on the same machine, which trusts that root, worked (#517).
//! ureq's own `native-certs` swaps the shipped list for the system's rather
//! than adding to it, and Android gives a process no system store to read,
//! so that would have failed every download on a phone instead.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// An agent with these timeouts that trusts the shipped roots and the
/// system's: for a file whose digest Concat already knows - a model, whose
/// sums are compiled in, or an installer or sound checked against a
/// manifest fetched with [`strict_agent`].
pub fn agent(connect: Duration, read: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(connect)
        .timeout_read(read)
        .tls_config(Arc::clone(tls()))
        .build()
}

/// An agent that trusts the shipped roots alone, for the manifests that
/// say what a download must hash to: the update manifest and the sound
/// list. A root the system was given by a proxy or an antivirus is enough
/// to re-sign github.com, and a manifest fetched through it could name a
/// swapped installer's digest beside the swapped installer. Behind such a
/// network the update check and the sound list fail, as every download did
/// before #517; the files they vouch for still come over [`agent`].
pub fn strict_agent(connect: Duration, read: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(connect)
        .timeout_read(read)
        .tls_config(Arc::clone(shipped_tls()))
        .build()
}

/// The TLS configuration [`strict_agent`] shares: the shipped roots only.
fn shipped_tls() -> &'static Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let config = rustls::ClientConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
        .expect("ring offers cipher suites for TLS 1.2 and 1.3")
        .with_root_certificates(rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        })
        .with_no_client_auth();
        Arc::new(config)
    })
}

/// The TLS configuration every agent shares. Built once: reading the
/// system's store takes a keychain query on macOS and a registry walk on
/// Windows, which is not worth paying per request.
fn tls() -> &'static Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG.get_or_init(|| {
        // ring, as ureq builds its own default with: ureq turns that
        // provider on, and naming it here does not depend on which
        // provider the process installed as its default, if any.
        let config = rustls::ClientConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
        .expect("ring offers cipher suites for TLS 1.2 and 1.3")
        .with_root_certificates(roots())
        .with_no_client_auth();
        Arc::new(config)
    })
}

/// The shipped roots, and the system's beside them.
fn roots() -> rustls::RootCertStore {
    let mut roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let system = rustls_native_certs::load_native_certs();
    // A store that cannot be read, in whole or part, leaves the shipped
    // roots, which is where every download stood before.
    if let Some(error) = system.errors.first() {
        log::debug!("network: the system's root certificates were not all read: {error}");
    }
    let (added, _) = roots.add_parsable_certificates(system.certs);
    log::debug!("network: {added} of the system's root certificates trusted beside Concat's");
    roots
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped roots are always there, whatever the system's store
    /// gave: a machine whose store is empty or unreadable is no worse off
    /// than before.
    #[test]
    fn the_shipped_roots_are_always_trusted() {
        assert!(roots().len() >= webpki_roots::TLS_SERVER_ROOTS.len());
        // The configuration is whole: a connection can be started on it.
        // The name is never looked up.
        let name = rustls::pki_types::ServerName::try_from("github.com").expect("a valid name");
        assert!(rustls::ClientConnection::new(Arc::clone(tls()), name.clone()).is_ok());
        assert!(rustls::ClientConnection::new(Arc::clone(shipped_tls()), name).is_ok());
    }
}
