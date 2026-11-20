//! The model connection: one OpenAI-compatible `chat/completions` request with tools (Ollama,
//! LM Studio, OpenAI and compatible services). Blocking; the panel runs it on a worker thread.
//!
//! The API key goes only into the `Authorization` header: never into errors, logs or files
//! (storing it is [`crate::settings`]' job).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde_json::{Value, json};
use studiocraft_ui::i18n::{tr, trf};

/// Longest reply read from the endpoint (8 MiB).
const MAX_REPLY: u64 = 8 << 20;
/// Longest error text shown from the endpoint.
const MAX_ERROR_TEXT: usize = 300;

/// Where requests go.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Endpoint {
    /// API root ending before `chat/completions`, e.g. `http://localhost:11434/v1`.
    pub url: String,
    pub model: String,
    /// Bearer token; none for local servers without one.
    pub key: Option<String>,
}

impl Endpoint {
    /// The URL checked: `http` or `https`, with a host; plain `http` only to this computer,
    /// so a key and the project never cross the network unencrypted.
    pub fn check(&self) -> Result<(), String> {
        let Some(host) = host(&self.url) else {
            return Err(tr("Die Adresse muss mit http:// oder https:// beginnen.").into());
        };
        if self.url.starts_with("http://") && !is_loopback(&host) {
            return Err(tr("http:// nur zu diesem Rechner; sonst https:// verwenden.").into());
        }
        if self.model.trim().is_empty() {
            return Err(tr("Kein Modell gewählt.").into());
        }
        if self.url.trim().starts_with("https://") && roots().is_empty() {
            return Err(tr("Keine Zertifikate des Systems gefunden.").into());
        }
        Ok(())
    }

    /// Whether the data stays on this computer.
    pub fn is_local(&self) -> bool {
        host(&self.url).is_some_and(|h| is_loopback(&h))
    }

    fn api(&self, path: &str) -> String {
        format!("{}/{path}", self.url.trim().trim_end_matches('/'))
    }
}

/// The host of an `http(s)://` URL, lower case and without port or brackets.
pub fn host(url: &str) -> Option<String> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // `user:pass@` would put a password into settings and the consent text.
    if authority.contains('@') {
        return None;
    }
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// One completion: `messages` (OpenAI form) with `tools` (OpenAI function form). Returns the
/// reply message (`role`, `content`, `tool_calls`).
pub fn chat(endpoint: &Endpoint, messages: &[Value], tools: &[Value]) -> Result<Value, String> {
    endpoint.check()?;
    let mut body = json!({
        "model": endpoint.model.trim(),
        "messages": messages,
        "temperature": 0.2,
    });
    if !tools.is_empty()
        && let Some(o) = body.as_object_mut()
    {
        o.insert("tools".into(), json!(tools));
    }
    let reply = post(endpoint, &endpoint.api("chat/completions"), &body)?;
    match reply
        .pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
    {
        Some("length") => {
            return Err(tr("Die Antwort wurde abgeschnitten (Grenze des Modells).").into());
        }
        Some("content_filter") => return Err(tr("Der Dienst hat die Antwort gefiltert.").into()),
        _ => {}
    }
    reply
        .pointer("/choices/0/message")
        .filter(|m| m.is_object())
        .cloned()
        .ok_or_else(|| tr("Der Endpunkt hat keine Nachricht geschickt.").into())
}

/// The models the endpoint offers (`GET models`): a check that sends no project data.
pub fn models(endpoint: &Endpoint) -> Result<Vec<String>, String> {
    let probe = Endpoint {
        model: "-".into(),
        ..endpoint.clone()
    };
    probe.check()?;
    let mut req = agent(&probe.url).get(probe.api("models"));
    if let Some(key) = key(endpoint) {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = req.call().map_err(transport_error)?;
    let v = read(resp)?;
    let mut ids: Vec<String> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["id"].as_str().map(str::to_owned))
        .collect();
    ids.sort();
    Ok(ids)
}

fn key(endpoint: &Endpoint) -> Option<&str> {
    endpoint
        .key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
}

fn post(endpoint: &Endpoint, url: &str, body: &Value) -> Result<Value, String> {
    let mut req = agent(url)
        .post(url)
        .header("Content-Type", "application/json");
    if let Some(key) = key(endpoint) {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    // As bytes, so the request has a length (some local servers reject chunked bodies).
    let bytes = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let resp = req.send(&bytes[..]).map_err(transport_error)?;
    read(resp)
}

/// The JSON body of `resp`, or the endpoint's error in words.
fn read(mut resp: ureq::http::Response<ureq::Body>) -> Result<Value, String> {
    let status = resp.status();
    let text = resp
        .body_mut()
        .with_config()
        .limit(MAX_REPLY)
        .read_to_string()
        .map_err(|e| trf("Antwort nicht lesbar: {fehler}", &[("fehler", &e)]))?;
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .or_else(|| v.get("error"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_default();
        let what = match status.as_u16() {
            401 | 403 => tr("Der Endpunkt hat den Schlüssel abgelehnt").to_owned(),
            404 => tr("Endpunkt oder Modell nicht gefunden").to_owned(),
            429 => tr("Zu viele Anfragen oder Kontingent aufgebraucht").to_owned(),
            c => format!("HTTP {c}"),
        };
        return Err(if detail.is_empty() {
            what
        } else {
            format!("{what}: {}", shorten(&detail, MAX_ERROR_TEXT))
        });
    }
    serde_json::from_str(&text).map_err(|e| {
        trf(
            "Der Endpunkt hat kein JSON geschickt: {fehler}",
            &[("fehler", &e)],
        )
    })
}

fn transport_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => tr("Keine Antwort in der Zeit.").into(),
        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound => {
            tr("Endpunkt nicht erreichbar (läuft der Server?).").into()
        }
        ureq::Error::Io(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::AddrNotAvailable
            ) =>
        {
            tr("Endpunkt nicht erreichbar (läuft der Server?).").into()
        }
        other => other.to_string(),
    }
}

/// `text` cut to at most `max` characters.
pub fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_owned(),
    }
}

/// The operating system's root certificates, read once.
fn roots() -> &'static [ureq::tls::Certificate<'static>] {
    static ROOTS: OnceLock<Vec<ureq::tls::Certificate<'static>>> = OnceLock::new();
    ROOTS.get_or_init(|| {
        rustls_native_certs::load_native_certs()
            .certs
            .iter()
            .map(|c| ureq::tls::Certificate::from_der(c.as_ref()).to_owned())
            .collect()
    })
}

/// An agent for `url` with finite deadlines (a local model may think for minutes) and, for
/// `https`, TLS by rustls with the operating system's certificates (read only then); error
/// statuses come back as responses to be read.
fn agent(url: &str) -> ureq::Agent {
    let certs = if url.trim().starts_with("https://") {
        roots()
    } else {
        &[]
    };
    let tls = ureq::tls::TlsConfig::builder()
        .provider(ureq::tls::TlsProvider::Rustls)
        .root_certs(ureq::tls::RootCerts::new_with_certs(certs))
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    // This computer is reached directly, never through a proxy from the environment.
    let local = host(url).is_some_and(|h| is_loopback(&h));
    let proxy = if local {
        None
    } else {
        ureq::Proxy::try_from_env()
    };
    ureq::Agent::config_builder()
        .proxy(proxy)
        .tls_config(tls)
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_send_body(Some(Duration::from_secs(60)))
        .timeout_recv_response(Some(Duration::from_secs(600)))
        .timeout_recv_body(Some(Duration::from_secs(120)))
        .build()
        .into()
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn hosts_and_what_counts_as_local() {
        assert_eq!(
            host("http://localhost:11434/v1").as_deref(),
            Some("localhost")
        );
        assert_eq!(
            host("https://API.example.com/v1").as_deref(),
            Some("api.example.com")
        );
        assert_eq!(host("http://[::1]:8080/v1").as_deref(), Some("::1"));
        assert_eq!(host("ftp://example.com"), None);
        assert_eq!(host("https://user:pw@example.com"), None);
        assert_eq!(host("https://"), None);
        let ep = |url: &str| Endpoint {
            url: url.into(),
            model: "m".into(),
            key: None,
        };
        assert!(ep("http://127.0.0.1:1234/v1").is_local());
        assert!(!ep("https://api.example.com/v1").is_local());
        assert!(ep("http://localhost:11434/v1").check().is_ok());
        assert!(ep("https://api.example.com/v1").check().is_ok());
        // The key would cross the network in clear text.
        assert!(ep("http://api.example.com/v1").check().is_err());
        assert!(
            Endpoint {
                model: " ".into(),
                ..ep("http://localhost/v1")
            }
            .check()
            .is_err()
        );
    }

    /// A one-request HTTP server answering `status` and `body`; returns its URL and the
    /// request it saw.
    fn serve_once(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/v1",
            listener.local_addr().unwrap().port()
        );
        let t = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut head = String::new();
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
                head.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut body_in = vec![0; length];
            reader.read_exact(&mut body_in).unwrap();
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            head + &String::from_utf8(body_in).unwrap()
        });
        (url, t)
    }

    #[test]
    fn a_chat_request_carries_messages_tools_and_key() {
        let (url, server) = serve_once(
            200,
            r#"{"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"inspect_project","arguments":"{}"}}]}}]}"#,
        );
        let ep = Endpoint {
            url,
            model: "test-model".into(),
            key: Some("sk-secret".into()),
        };
        let tools = [json!({"type": "function", "function": {"name": "inspect_project"}})];
        let msg = chat(&ep, &[json!({"role": "user", "content": "Hallo"})], &tools).unwrap();
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "inspect_project");
        let request = server.join().unwrap();
        assert!(
            request.starts_with("POST /v1/chat/completions"),
            "{request}"
        );
        assert!(request.contains("Bearer sk-secret"));
        assert!(request.contains(r#""model":"test-model""#));
        assert!(request.contains(r#""tools":"#));
        assert!(request.contains("Hallo"));
    }

    #[test]
    fn endpoint_errors_are_words_without_the_key() {
        let (url, server) = serve_once(401, r#"{"error":{"message":"Incorrect API key"}}"#);
        let ep = Endpoint {
            url,
            model: "m".into(),
            key: Some("sk-secret".into()),
        };
        let e = chat(&ep, &[], &[]).unwrap_err();
        server.join().unwrap();
        assert!(
            e.contains("Schlüssel abgelehnt") && e.contains("Incorrect API key"),
            "{e}"
        );
        assert!(!e.contains("sk-secret"));

        let (url, server) = serve_once(
            200,
            r#"{"choices":[{"finish_reason":"length","message":{"content":"ha"}}]}"#,
        );
        let e = chat(&Endpoint { url, ..ep.clone() }, &[], &[]).unwrap_err();
        server.join().unwrap();
        assert!(e.contains("abgeschnitten"), "{e}");

        let (url, server) = serve_once(200, "not json");
        assert!(chat(&Endpoint { url, ..ep.clone() }, &[], &[]).is_err());
        server.join().unwrap();

        // Nobody listening.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let e = chat(
            &Endpoint {
                url: format!("http://127.0.0.1:{port}/v1"),
                ..ep
            },
            &[],
            &[],
        )
        .unwrap_err();
        assert!(e.contains("nicht erreichbar"), "{e}");
    }

    #[test]
    fn models_lists_the_ids() {
        let (url, server) = serve_once(200, r#"{"data":[{"id":"b"},{"id":"a"}]}"#);
        let ids = models(&Endpoint {
            url,
            model: String::new(),
            key: None,
        })
        .unwrap();
        assert_eq!(ids, ["a", "b"]);
        assert!(server.join().unwrap().starts_with("GET /v1/models"));
    }

    #[test]
    fn shorten_cuts_on_characters() {
        assert_eq!(shorten("äöü", 2), "äö…");
        assert_eq!(shorten("ab", 2), "ab");
    }
}
