//! Instance principals: a program on an OCI instance signs as the instance itself, with no API
//! key on disk. Permissions come from a dynamic group that matches the instance.
//!
//! The instance metadata service (`http://169.254.169.254/opc/v2`) hands out a certificate and its
//! key; OCI's X.509 federation (`POST https://auth.<region>.oraclecloud.com/v1/x509`) trades them
//! for a session token; requests are then signed with `keyId="ST$<token>"`. The token lasts about
//! 20 minutes and is renewed shortly before it runs out.
//!
//! The session key is the instance's own key from the metadata service: `ring` cannot generate
//! RSA keys, and the session key only has to be one the caller holds.

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64URL};

use crate::client::{Client, Method};
use crate::error::{Error, Result};
use crate::signer::{Signer, pem_der};

/// Where the instance metadata service answers.
pub const DEFAULT_METADATA_ENDPOINT: &str = "http://169.254.169.254/opc/v2";

/// Renew the session token when it has less than this left.
const RENEW_BEFORE: Duration = Duration::from_secs(5 * 60);
/// If a token's expiry cannot be read, assume this lifetime (OCI's is about 20 minutes).
const ASSUMED_LIFETIME: Duration = Duration::from_secs(15 * 60);

/// What the metadata service says about this instance.
pub(crate) struct Identity {
    /// e.g. `eu-amsterdam-1`.
    pub region: String,
    /// From the certificate's `OU=opc-tenant:...`.
    pub tenancy: String,
}

/// Signs as the instance, renewing the session token as needed.
pub(crate) struct InstancePrincipal {
    metadata: String,
    agent: ureq::Agent,
    /// The region the instance is in: federation happens there.
    region: String,
    tenancy: String,
    session: Mutex<Option<Session>>,
}

struct Session {
    signer: Arc<Signer>,
    expires: SystemTime,
}

impl InstancePrincipal {
    /// Read the region and tenancy from the metadata service.
    pub(crate) fn new(metadata: String, agent: ureq::Agent) -> Result<(Self, Identity)> {
        let region = metadata_get(&agent, &metadata, "/instance/canonicalRegionName")?
            .trim()
            .to_owned();
        let cert = metadata_get(&agent, &metadata, "/identity/cert.pem")?;
        let tenancy = tenancy_of(&pem_der(&cert).map_err(|e| fail(format!("certificate: {e}")))?)?;
        let me = Self {
            metadata,
            agent,
            region: region.clone(),
            tenancy: tenancy.clone(),
            session: Mutex::new(None),
        };
        Ok((me, Identity { region, tenancy }))
    }

    /// The signer for the current session, renewing it first if it is about to run out.
    pub(crate) fn signer(&self, client: &Client) -> Result<Arc<Signer>> {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = session.as_ref()
            && s.expires > SystemTime::now() + RENEW_BEFORE
        {
            return Ok(Arc::clone(&s.signer));
        }
        let fresh = self.federate(client)?;
        let signer = Arc::clone(&fresh.signer);
        *session = Some(fresh);
        Ok(signer)
    }

    /// Trade the instance certificate for a session token.
    fn federate(&self, client: &Client) -> Result<Session> {
        let cert_pem = metadata_get(&self.agent, &self.metadata, "/identity/cert.pem")?;
        let key_pem = metadata_get(&self.agent, &self.metadata, "/identity/key.pem")?;
        let intermediate_pem =
            metadata_get(&self.agent, &self.metadata, "/identity/intermediate.pem")?;
        let cert = pem_der(&cert_pem).map_err(|e| fail(format!("certificate: {e}")))?;
        let intermediate = pem_der(&intermediate_pem)
            .map_err(|e| fail(format!("intermediate certificate: {e}")))?;

        let key_id = format!("{}/fed-x509/{}", self.tenancy, fingerprint(&cert));
        let instance = Signer::from_pem(key_id, &key_pem).map_err(|e| fail(format!("key: {e}")))?;
        let body = serde_json::json!({
            "certificate": BASE64.encode(&cert),
            "publicKey": BASE64.encode(instance.public_key_spki()),
            "intermediateCertificates": [BASE64.encode(&intermediate)],
            "purpose": "DEFAULT",
        });
        let body = serde_json::to_vec(&body).expect("the request always serializes");
        let answer: serde_json::Value = client
            .in_region(self.region.clone())
            .send_signed(
                &instance,
                Method::Post,
                "auth",
                "/v1/x509",
                Some(&body),
                &[],
            )?
            .json()?;
        let token = answer["token"]
            .as_str()
            .ok_or_else(|| fail("the federation answer has no token".to_owned()))?;

        let signer = Signer::from_pem(format!("ST${token}"), &key_pem)
            .map_err(|e| fail(format!("key: {e}")))?;
        Ok(Session {
            signer: Arc::new(signer),
            expires: expiry(token).unwrap_or_else(|| SystemTime::now() + ASSUMED_LIFETIME),
        })
    }
}

fn fail(reason: String) -> Error {
    Error::Transport(format!("instance principal: {reason}"))
}

/// GET from the metadata service, which wants `Authorization: Bearer Oracle`.
fn metadata_get(agent: &ureq::Agent, base: &str, path: &str) -> Result<String> {
    let url = format!("{base}{path}");
    let mut response = agent
        .get(&url)
        .header("Authorization", "Bearer Oracle")
        .call()
        .map_err(|e| {
            fail(format!(
                "cannot reach the instance metadata service ({e}) — is this an OCI instance?"
            ))
        })?;
    let status = response.status().as_u16();
    let text = response
        .body_mut()
        .read_to_string()
        .map_err(|e| fail(format!("{url}: {e}")))?;
    if !(200..300).contains(&status) {
        return Err(fail(format!("{url} answered {status}")));
    }
    Ok(text)
}

/// The tenancy OCID from the certificate subject's `OU=opc-tenant:<ocid>`. The value is found
/// in the DER by its prefix; the byte before it is the string's length.
fn tenancy_of(cert: &[u8]) -> Result<String> {
    const PREFIX: &[u8] = b"opc-tenant:";
    let at = cert
        .windows(PREFIX.len())
        .position(|w| w == PREFIX)
        .ok_or_else(|| fail("the certificate names no tenancy (opc-tenant)".to_owned()))?;
    let len = at
        .checked_sub(1)
        .map(|i| cert[i] as usize)
        .filter(|len| *len < 0x80 && *len > PREFIX.len())
        .ok_or_else(|| fail("cannot read the tenancy from the certificate".to_owned()))?;
    let value = cert
        .get(at + PREFIX.len()..at + len)
        .and_then(|v| std::str::from_utf8(v).ok())
        .filter(|v| v.starts_with("ocid1.tenancy."))
        .ok_or_else(|| fail("cannot read the tenancy from the certificate".to_owned()))?;
    Ok(value.to_owned())
}

/// SHA-1 of the certificate, `AB:CD:...` — what the federation `keyId` names.
fn fingerprint(cert: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, cert)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// The `exp` of a JWT, unverified — only to know when to renew.
fn expiry(token: &str) -> Option<SystemTime> {
    let payload = token.split('.').nth(1)?;
    let claims: serde_json::Value =
        serde_json::from_slice(&BASE64URL.decode(payload).ok()?).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(claims["exp"].as_u64()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CERT: &str = include_str!("../testdata/instance-cert.pem");

    #[test]
    fn tenancy_and_fingerprint_of_the_certificate() {
        let der = pem_der(CERT).unwrap();
        assert_eq!(tenancy_of(&der).unwrap(), "ocid1.tenancy.oc1..t");
        // openssl x509 -in testdata/instance-cert.pem -noout -fingerprint -sha1
        assert_eq!(
            fingerprint(&der),
            "E2:AB:FE:2A:47:A6:7E:BA:29:BE:F8:B7:83:60:26:B8:76:EA:24:66"
        );
        assert!(tenancy_of(b"no tenancy here").is_err());
    }

    #[test]
    fn token_expiry() {
        let payload = BASE64URL.encode(br#"{"exp":1794326700,"sub":"x"}"#);
        assert_eq!(
            expiry(&format!("h.{payload}.s")),
            Some(UNIX_EPOCH + Duration::from_secs(1_794_326_700))
        );
        assert_eq!(expiry("not-a-jwt"), None);
    }
}
