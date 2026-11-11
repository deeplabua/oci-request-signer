//! Sign Oracle Cloud Infrastructure (OCI) REST API requests — no OCI SDK, no OpenSSL.
//!
//! - [`config`]: reads `~/.oci/config` ([`Profile`])
//! - [`signer`]: OCI request signatures, draft-cavage HTTP Signatures with RSA-SHA256 ([`Signer`])
//! - [`client`]: a small blocking client on `ureq` + `rustls` ([`Client`]; feature `client`,
//!   on by default)
//!
//! The crate knows nothing about individual OCI services: it reads the config, signs, sends
//! and turns error responses into [`ApiError`]. Service endpoints and types are the caller's.
//!
//! ```no_run
//! # #[cfg(feature = "client")]
//! # fn main() -> oci_request_signer::Result<()> {
//! use oci_request_signer::{Client, DEFAULT_PROFILE, Profile, encode};
//!
//! let profile = Profile::load(&Profile::default_path().unwrap(), DEFAULT_PROFILE)?;
//! let client = Client::builder(&profile)
//!     .user_agent(concat!("my-tool/", env!("CARGO_PKG_VERSION")))
//!     .endpoint(std::env::var("MY_TOOL_ENDPOINT").ok())
//!     .build()?;
//! let path = format!("/20160918/instances?compartmentId={}", encode(client.tenancy()));
//! let instances: serde_json::Value = client.get("iaas", &path)?;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "client"))]
//! # fn main() {}
//! ```
//!
//! Without the `client` feature the crate is config + signer only: build the request with
//! your own HTTP stack and add the headers [`Signer::sign`] returns.
//!
//! Not to be confused with signing OCI *container* artifacts (cosign, notation).

#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(feature = "client")]
#[cfg_attr(docsrs, doc(cfg(feature = "client")))]
pub mod client;
pub mod config;
mod encode;
mod error;
#[cfg(feature = "client")]
mod instance;
pub mod signer;

#[cfg(feature = "client")]
pub use client::{Client, ClientBuilder, DEFAULT_DOMAIN, Method, OCI_DOMAIN, Response};
pub use config::{DEFAULT_PROFILE, Profile, expand_home};
pub use encode::encode;
pub use error::{ApiError, Error, Result};
#[cfg(feature = "client")]
pub use instance::DEFAULT_METADATA_ENDPOINT;
pub use signer::Signer;
