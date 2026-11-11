<p align="center"><img src=".github/oci-request-signer-256.png" width="128" height="128" alt="oci-request-signer icon"></p>

# oci-request-signer

[![CI](https://img.shields.io/github/actions/workflow/status/deeplabua/oci-request-signer/ci.yml?branch=main&logo=github&label=CI)](https://github.com/deeplabua/oci-request-signer/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/oci-request-signer?label=crates.io&color=e37933)](https://crates.io/crates/oci-request-signer)
[![docs.rs](https://img.shields.io/docsrs/oci-request-signer?label=docs.rs)](https://docs.rs/oci-request-signer)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Stars](https://img.shields.io/github/stars/deeplabua/oci-request-signer?style=flat&logo=github&label=Star)](https://github.com/deeplabua/oci-request-signer/stargazers)

**Sign Oracle Cloud (OCI) REST API requests from Rust — no OCI SDK, no OpenSSL.**

Reads `~/.oci/config`, signs requests the way OCI expects (draft-cavage HTTP
Signatures, RSA-SHA256) and sends them with a small blocking client on `ureq` + `rustls`.

Not to be confused with signing OCI *container* artifacts (cosign, notation): this crate is
about **Oracle Cloud Infrastructure** API requests.

## Use

```toml
[dependencies]
oci-request-signer = "0.2"
```

```rust,no_run
use oci_request_signer::{Client, DEFAULT_PROFILE, Profile, encode};

let profile = Profile::load(&Profile::default_path().unwrap(), DEFAULT_PROFILE)?;
let client = Client::builder(&profile)
    .user_agent(concat!("my-tool/", env!("CARGO_PKG_VERSION")))
    .endpoint(std::env::var("MY_TOOL_ENDPOINT").ok()) // a local mock in tests; None = real OCI
    .retry_throttled(3) // retry a 429 after its Retry-After; off by default
    .build()?;

// GET https://iaas.<region>.oraclecloud.com/20160918/instances?compartmentId=...
let path = format!("/20160918/instances?compartmentId={}", encode(client.tenancy()));
let instances: serde_json::Value = client.get("iaas", &path)?;

// Every page of a list (opc-next-page), whether it answers [...] or {"items": [...]}.
let all: Vec<serde_json::Value> = client.list("iaas", &path)?;

// Raw responses: status, headers (opc-next-page, etag, ...), empty 204 bodies.
let resp = client.delete("iaas", "/20160918/bootVolumeBackups/ocid1...", &[])?;
assert_eq!(resp.status(), 204);

// Another region with the same key.
let phoenix = client.in_region("us-phoenix-1");

// Newer services (Usage API, Organizations, Limits, Budgets) live on oci.oraclecloud.com.
let usage = client.on_domain(oci_request_signer::OCI_DOMAIN);
let query = serde_json::json!({ "tenantId": client.tenancy(), "granularity": "MONTHLY",
    "queryType": "COST", "timeUsageStarted": "2026-10-01T00:00:00Z",
    "timeUsageEnded": "2026-11-01T00:00:00Z" });
let cost: serde_json::Value = usage.post("usageapi", "/20200107/usage", &query, &[])?;
# Ok::<(), oci_request_signer::Error>(())
```

The crate knows no OCI services: you pass the host prefix (`iaas`, `identity`, `telemetry`,
`usageapi`, `objectstorage`, ...), pick the domain for the newer ones, and give the path from the
[API reference](https://docs.oracle.com/iaas/api/). Errors from OCI come back as `ApiError`
with the status, OCI error code, message, `opc-request-id` and, for a 429, `Retry-After`.
Server errors (5xx) are never retried by the crate: a `POST` may already have happened.

### Your own HTTP stack

With `default-features = false` the crate is config + signer only (`base64`, `dirs`, `ring`,
`thiserror`): build the request yourself — `reqwest`, async, anything — and add the headers
`Signer::sign` returns.

## Not yet

Instance / resource principals and session tokens, encrypted keys, async. See [`CHANGELOG.md`](CHANGELOG.md).

## Used by

- [oci-arm-catcher](https://github.com/deeplabua/oci-arm-catcher) — grabs a free Ampere A1 instance
  the moment capacity appears;
- [oci-free-keeper](https://github.com/deeplabua/oci-free-keeper) — knows the moment an instance is
  stopped or gone, and how close it is to Oracle's idle rule;
- [oci-free-audit](https://github.com/deeplabua/oci-free-audit) — will Oracle charge me? A read-only
  check against the Always Free limits and the actual bill.

All part of [DeepLab](https://deeplab.tools).

## Support

If `oci-request-signer` saved you from writing OCI request signing yourself, two things help a lot:

- ⭐ **[Star the repo](https://github.com/deeplabua/oci-request-signer)** — it's how other Rust developers talking to Oracle Cloud find it.
- ☕ **[Buy me a coffee](https://send.monobank.ua/jar/4Hq7auheaa)** — if it saved you some time and you feel like it.

Both are completely optional. The crate is free and always will be.

## License

[MIT](LICENSE)
