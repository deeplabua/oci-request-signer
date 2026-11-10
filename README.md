# oci-request-signer

**Sign Oracle Cloud (OCI) REST API requests from Rust — no OCI SDK, no OpenSSL.**

Reads `~/.oci/config`, signs requests the way OCI expects (draft-cavage HTTP
Signatures, RSA-SHA256) and sends them with a small blocking client on `ureq` + `rustls`.

Not to be confused with signing OCI *container* artifacts (cosign, notation): this crate is
about **Oracle Cloud Infrastructure** API requests.

## Use

```toml
[dependencies]
oci-request-signer = "0.1"
```

```rust,no_run
use oci_request_signer::{Client, DEFAULT_PROFILE, Profile, encode};

let profile = Profile::load(&Profile::default_path().unwrap(), DEFAULT_PROFILE)?;
let client = Client::builder(&profile)
    .user_agent(concat!("my-tool/", env!("CARGO_PKG_VERSION")))
    .endpoint(std::env::var("MY_TOOL_ENDPOINT").ok()) // a local mock in tests; None = real OCI
    .build()?;

// GET https://iaas.<region>.oraclecloud.com/20160918/instances?compartmentId=...
let path = format!("/20160918/instances?compartmentId={}", encode(client.tenancy()));
let instances: serde_json::Value = client.get("iaas", &path)?;

// Raw responses: status, headers (opc-next-page, etag, ...), empty 204 bodies.
let resp = client.delete("iaas", "/20160918/bootVolumeBackups/ocid1...", &[])?;
assert_eq!(resp.status(), 204);

// Another region with the same key.
let phoenix = client.in_region("us-phoenix-1");
# Ok::<(), oci_request_signer::Error>(())
```

The crate knows no OCI services: you pass the host prefix (`iaas`, `identity`, `telemetry`,
`usageapi`, `objectstorage`, ...) and the path from the
[API reference](https://docs.oracle.com/iaas/api/). Errors from OCI come back as `ApiError`
with the status, OCI error code, message and `opc-request-id`.

### Your own HTTP stack

With `default-features = false` the crate is config + signer only (`base64`, `dirs`, `ring`,
`thiserror`): build the request yourself — `reqwest`, async, anything — and add the headers
`Signer::sign` returns.

## Not yet

Instance / resource principals and session tokens, encrypted keys, async, pagination and
retry helpers. See [`CHANGELOG.md`](CHANGELOG.md).

## Used by

[oci-arm-catcher](https://github.com/deeplabua/oci-arm-catcher), oci-free-keeper and
oci-free-audit — part of [DeepLab](https://deeplab.tools).

## License

[MIT](LICENSE)
