//! Query-string encoding.

/// Percent-encode a query value (RFC 3986 unreserved characters pass through).
///
/// OCIDs, availability domain names and MQL queries all need it:
///
/// ```
/// assert_eq!(oci_request_signer::encode("Abcd:EU-AMSTERDAM-1-AD-1"), "Abcd%3AEU-AMSTERDAM-1-AD-1");
/// ```
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_query_values() {
        assert_eq!(
            encode("Abcd:EU-AMSTERDAM-1-AD-1"),
            "Abcd%3AEU-AMSTERDAM-1-AD-1"
        );
        assert_eq!(encode("Canonical Ubuntu"), "Canonical%20Ubuntu");
        assert_eq!(encode("ocid1.tenancy.oc1..aa"), "ocid1.tenancy.oc1..aa");
        assert_eq!(encode("CpuUtilization[1h]"), "CpuUtilization%5B1h%5D");
    }
}
