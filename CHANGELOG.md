# Changelog

## 0.1.2 — 2026-11-10

- README: icon and badges (CI, crates.io, license, stars).
- README: a Support section — star the repo, or buy me a coffee.

## 0.1.1 — 2026-11-10

- Domains: `ClientBuilder::domain`, `Client::on_domain`, `Client::domain` and `Client::host`,
  with `DEFAULT_DOMAIN` (`oraclecloud.com`) and `OCI_DOMAIN` (`oci.oraclecloud.com`).
  Newer services — Usage API, Organizations, Limits, Budgets — answer only on
  `<service>.<region>.oci.oraclecloud.com`; reach them with
  `client.on_domain(OCI_DOMAIN)`. Other realms: `ClientBuilder::domain`.

## 0.1.0 — 2026-11-10

First release: the config, signer and client code from
[oci-arm-catcher-core](https://crates.io/crates/oci-arm-catcher-core) 0.1.3, with
nothing tied to oci-arm-catcher. Signing behaviour is unchanged.

- `Profile`: reads `~/.oci/config`, every profile inherits `[DEFAULT]`.
- `Signer`: draft-cavage HTTP Signatures with RSA-SHA256 via `ring`; PKCS#8 and PKCS#1
  keys; encrypted keys get an error that says how to decrypt them.
- `Client` (feature `client`, on by default): blocking, `ureq` + `rustls`.
  - `Client::builder`: user agent, endpoint override for mocks, region, timeouts.
    The crate reads no environment variables.
  - `request(Method, ...)` returns a `Response` with status, headers and raw body,
    so `204 No Content` and `opc-next-page` work; non-2xx answers are `Error::Api`.
  - `get`, `post`, `put` (JSON) and `delete`; `POST` / `PUT` without a body sign an
    empty one, as OCI requires.
  - `in_region` reuses the loaded key for another region.
- `encode`: percent-encodes query values.
- `default-features = false` leaves config + signer only (`base64`, `dirs`, `ring`,
  `thiserror`) for callers with their own HTTP stack.
