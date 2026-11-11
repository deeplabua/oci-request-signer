//! The client against a local mock server: what goes on the wire and how answers come back.
#![cfg(feature = "client")]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use oci_request_signer::{Client, Error, Method, Profile};

/// One request as the mock saw it.
#[derive(Debug)]
struct Seen {
    request_line: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Serve one canned `response` (full HTTP text) and report the request.
fn mock(response: &'static str) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let (k, v) = line.split_once(':').unwrap();
            headers.push((k.trim().to_owned(), v.trim().to_owned()));
        }
        let len: usize = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .map_or(0, |(_, v)| v.parse().unwrap());
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        let mut stream = stream;
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
        tx.send(Seen {
            request_line: request_line.trim_end().to_owned(),
            headers,
            body,
        })
        .unwrap();
    });
    (base, rx)
}

fn profile() -> Profile {
    Profile {
        user: "ocid1.user.oc1..u".into(),
        fingerprint: "aa:bb".into(),
        tenancy: "ocid1.tenancy.oc1..t".into(),
        region: "eu-amsterdam-1".into(),
        key_file: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/test-key-pkcs8.pem"),
        pass_phrase: None,
    }
}

fn client(base: String) -> Client {
    Client::builder(&profile())
        .user_agent("test-tool/1.0")
        .endpoint(base)
        .build()
        .unwrap()
}

#[derive(serde::Deserialize, Debug, PartialEq)]
struct Item {
    id: String,
}

#[test]
fn get_parses_json_and_signs() {
    let (base, rx) = mock(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 13\r\n\
         connection: close\r\n\r\n[{\"id\":\"a\"}]\n",
    );
    let items: Vec<Item> = client(base).get("iaas", "/20160918/x?a=1").unwrap();
    assert_eq!(items, vec![Item { id: "a".into() }]);

    let seen = rx.recv().unwrap();
    assert_eq!(seen.request_line, "GET /20160918/x?a=1 HTTP/1.1");
    assert_eq!(seen.header("user-agent"), Some("test-tool/1.0"));
    assert!(seen.header("date").is_some());
    let auth = seen.header("authorization").unwrap();
    assert!(auth.contains(r#"keyId="ocid1.tenancy.oc1..t/ocid1.user.oc1..u/aa:bb""#));
    assert!(auth.contains(r#"headers="date (request-target) host""#));
    assert!(seen.header("x-content-sha256").is_none());
}

#[test]
fn delete_accepts_204_without_body() {
    let (base, rx) =
        mock("HTTP/1.1 204 No Content\r\nopc-request-id: req-9\r\nconnection: close\r\n\r\n");
    let resp = client(base)
        .delete(
            "iaas",
            "/20160918/bootVolumeBackups/b1",
            &[("if-match", "e1")],
        )
        .unwrap();
    assert_eq!(resp.status(), 204);
    assert!(resp.body().is_empty());
    assert_eq!(resp.request_id(), Some("req-9"));

    let seen = rx.recv().unwrap();
    assert_eq!(
        seen.request_line,
        "DELETE /20160918/bootVolumeBackups/b1 HTTP/1.1"
    );
    assert_eq!(seen.header("if-match"), Some("e1"));
}

#[test]
fn post_without_body_signs_an_empty_one() {
    let (base, rx) = mock("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}");
    let resp = client(base)
        .request(
            Method::Post,
            "iaas",
            "/20160918/instances/i1?action=START",
            None,
            &[],
        )
        .unwrap();
    assert_eq!(resp.text().unwrap(), "{}");

    let seen = rx.recv().unwrap();
    assert!(seen.request_line.starts_with("POST "));
    assert_eq!(seen.header("content-length"), Some("0"));
    assert_eq!(
        seen.header("x-content-sha256"),
        Some("47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=")
    );
    assert!(seen.body.is_empty());
    assert!(
        seen.header("authorization")
            .unwrap()
            .contains("content-length content-type x-content-sha256")
    );
}

#[test]
fn post_sends_json_and_exposes_headers() {
    let (base, rx) = mock(
        "HTTP/1.1 200 OK\r\nOPC-Next-Page: p2\r\ncontent-length: 10\r\n\
         connection: close\r\n\r\n{\"id\":\"b\"}",
    );
    let c = client(base);
    let body = serde_json::json!({ "displayName": "x" });
    let resp = c
        .request(
            Method::Post,
            "usageapi",
            "/20200107/usage",
            Some(serde_json::to_vec(&body).unwrap().as_slice()),
            &[],
        )
        .unwrap();
    assert_eq!(resp.header("opc-next-page"), Some("p2"));
    assert_eq!(resp.json::<Item>().unwrap(), Item { id: "b".into() });

    let seen = rx.recv().unwrap();
    assert_eq!(seen.body, br#"{"displayName":"x"}"#);
    assert_eq!(seen.header("content-type"), Some("application/json"));
}

#[test]
fn error_status_becomes_api_error() {
    let (base, _rx) = mock(
        "HTTP/1.1 404 Not Found\r\nopc-request-id: req-404\r\ncontent-length: 51\r\n\
         connection: close\r\n\r\n{\"code\":\"NotAuthorizedOrNotFound\",\"message\":\"nope\"}",
    );
    let err = client(base)
        .get::<serde_json::Value>("iaas", "/x")
        .unwrap_err();
    let Error::Api(api) = err else {
        panic!("expected an API error, got {err:?}")
    };
    assert_eq!(api.status, 404);
    assert_eq!(api.code, "NotAuthorizedOrNotFound");
    assert_eq!(api.message, "nope");
    assert_eq!(api.request_id.as_deref(), Some("req-404"));
}

#[test]
fn builder_region_and_defaults() {
    let p = profile();
    let c = Client::new(&p).unwrap();
    assert_eq!(c.region(), "eu-amsterdam-1");
    assert_eq!(c.tenancy(), "ocid1.tenancy.oc1..t");
    assert_eq!(c.in_region("us-phoenix-1").region(), "us-phoenix-1");
    let c = Client::builder(&p)
        .region("uk-london-1")
        .endpoint(Some(String::new()))
        .build()
        .unwrap();
    assert_eq!(c.region(), "uk-london-1");
    assert!(format!("{c:?}").contains("endpoint: None"));
}

#[test]
fn missing_key_is_a_key_error() {
    let mut p = profile();
    p.key_file = PathBuf::from("/nonexistent/key.pem");
    assert!(matches!(Client::new(&p), Err(Error::Key { .. })));
}

#[test]
fn domains_build_the_host() {
    use oci_request_signer::{DEFAULT_DOMAIN, OCI_DOMAIN};
    let p = profile();
    let c = Client::new(&p).unwrap();
    assert_eq!(c.domain(), DEFAULT_DOMAIN);
    assert_eq!(c.host("iaas"), "iaas.eu-amsterdam-1.oraclecloud.com");

    let usage = c.on_domain(OCI_DOMAIN);
    assert_eq!(
        usage.host("usageapi"),
        "usageapi.eu-amsterdam-1.oci.oraclecloud.com"
    );
    assert_eq!(
        c.domain(),
        DEFAULT_DOMAIN,
        "on_domain leaves the original alone"
    );
    assert_eq!(
        usage.in_region("us-phoenix-1").host("limits"),
        "limits.us-phoenix-1.oci.oraclecloud.com"
    );

    let other_realm = Client::builder(&p)
        .domain("oraclegovcloud.com")
        .build()
        .unwrap();
    assert_eq!(
        other_realm.host("iaas"),
        "iaas.eu-amsterdam-1.oraclegovcloud.com"
    );
    assert!(format!("{other_realm:?}").contains("oraclegovcloud.com"));
}

#[test]
fn endpoint_override_wins_over_domain() {
    let (base, rx) = mock("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n[]");
    let c = client(base).on_domain(oci_request_signer::OCI_DOMAIN);
    let _: Vec<Item> = c.get("usageapi", "/x").unwrap();
    let seen = rx.recv().unwrap();
    assert!(seen.header("host").unwrap().starts_with("127.0.0.1:"));
}

/// Serve `responses` in order, one per connection (each says `Connection: close`), and report
/// every request.
fn mock_seq(responses: Vec<&'static str>) -> (String, mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for response in responses {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                let (k, v) = line.split_once(':').unwrap();
                headers.push((k.trim().to_owned(), v.trim().to_owned()));
            }
            let mut stream = stream;
            stream.write_all(response.as_bytes()).unwrap();
            stream.flush().unwrap();
            tx.send(Seen {
                request_line: request_line.trim_end().to_owned(),
                headers,
                body: Vec::new(),
            })
            .unwrap();
        }
    });
    (base, rx)
}

const THROTTLED: &str = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: 52\r\n\r\n{\"code\":\"TooManyRequests\",\"message\":\"slow down pls\"}";

#[test]
fn throttled_requests_are_retried_when_asked() {
    let (base, rx) = mock_seq(vec![
        THROTTLED,
        THROTTLED,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: 2\r\n\r\n[]",
    ]);
    let c = Client::builder(&profile())
        .endpoint(base)
        .retry_throttled(2)
        .build()
        .unwrap();
    let v: Vec<serde_json::Value> = c.get("iaas", "/20160918/instances").unwrap();
    assert!(v.is_empty());
    let seen: Vec<Seen> = rx.iter().take(3).collect();
    // Each attempt is signed afresh.
    assert!(seen.iter().all(|s| s.header("authorization").is_some()));
}

#[test]
fn throttling_is_reported_by_default() {
    let (base, _rx) = mock_seq(vec![THROTTLED]);
    let err = client(base)
        .get::<serde_json::Value>("iaas", "/x")
        .unwrap_err();
    match err {
        Error::Api(e) => {
            assert_eq!((e.status, e.code.as_str()), (429, "TooManyRequests"));
            assert_eq!(e.retry_after, Some(std::time::Duration::ZERO));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn lists_follow_next_page_and_accept_items_objects() {
    let (base, rx) = mock_seq(vec![
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nopc-next-page: p 2\r\nconnection: close\r\ncontent-length: 5\r\n\r\n[1,2]",
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: 13\r\n\r\n{\"items\":[3]}",
    ]);
    let all: Vec<u32> = client(base)
        .list("iaas", "/20160918/instances?compartmentId=c")
        .unwrap();
    assert_eq!(all, vec![1, 2, 3]);
    let seen: Vec<Seen> = rx.iter().take(2).collect();
    assert_eq!(
        seen[1].request_line,
        "GET /20160918/instances?compartmentId=c&page=p%202 HTTP/1.1"
    );
}

#[test]
fn endpoint_placeholders_name_the_region_and_service() {
    let (base, rx) = mock("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n[]");
    let c = client(format!("{base}/{{region}}/{{service}}")).in_region("eu-frankfurt-1");
    let _: Vec<Item> = c.get("iaas", "/20160918/instances?a=1").unwrap();
    let seen = rx.recv().unwrap();
    assert_eq!(
        seen.request_line,
        "GET /eu-frankfurt-1/iaas/20160918/instances?a=1 HTTP/1.1"
    );
    assert!(seen.header("host").unwrap().starts_with("127.0.0.1:"));
    assert!(!seen.header("host").unwrap().contains('/'));
}
