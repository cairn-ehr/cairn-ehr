//! The gateway's wire: pure byte parsing and encoding, no I/O (#909 slice B, design D2).
//!
//! The gateway reads only the first few messages of a Postgres connection (the opening
//! `SSLRequest`/`GSSENCRequest`, then the startup or cancel packet inside TLS) and writes only a
//! refusal. Everything here takes bytes in and gives bytes or a verdict out, so it is unit-tested
//! without a socket. Protocol reference:
//! https://www.postgresql.org/docs/current/protocol-message-formats.html

/// Magic "protocol code" of an `SSLRequest` (message length 8).
const SSL_CODE: u32 = 80_877_103;
/// Magic code of a `GSSENCRequest` (message length 8).
const GSS_CODE: u32 = 80_877_104;
/// Magic code of a `CancelRequest`.
const CANCEL_CODE: u32 = 80_877_102;
/// Largest startup packet accepted, whole message. Postgres itself caps it near 10 kB; refusing
/// larger ones stops an unauthenticated peer from making the gateway buffer without bound.
pub const MAX_STARTUP: usize = 10_000;
/// A cancel is length + code + process id (4) + key (4 for protocol 3.0, up to 256 for 3.2).
const CANCEL_MIN: usize = 16;
const CANCEL_MAX: usize = 268;

/// What the first 8 bytes of a connection ask for.
#[derive(Debug, PartialEq, Eq)]
pub enum Opening {
    /// `SSLRequest`: answer `S` and start TLS.
    Ssl,
    /// `GSSENCRequest`: answer `N`; the client must then send an `SSLRequest`.
    Gss,
    /// Anything else (plain startup, plain cancel, garbage): close without a reply.
    Other,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Classify the 8-byte opening message.
pub fn opening(first8: [u8; 8]) -> Opening {
    match (be32(&first8[..4]), be32(&first8[4..])) {
        (8, SSL_CODE) => Opening::Ssl,
        (8, GSS_CODE) => Opening::Gss,
        _ => Opening::Other,
    }
}

/// A parsed `StartupMessage`. `raw` is replayed upstream byte for byte; `params` is only for the
/// gateway's own checks (`user`, `database`).
#[derive(Debug, PartialEq, Eq)]
pub struct Startup {
    pub raw: Vec<u8>,
    pub params: Vec<(String, String)>,
}

impl Startup {
    /// The value of a startup parameter, if the client sent it.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// What came inside TLS: a startup to vet, or a cancel (the whole packet) to forward as is.
#[derive(Debug, PartialEq, Eq)]
pub enum Inside {
    Startup(Startup),
    Cancel(Vec<u8>),
}

/// Parse the NUL-terminated key/value body of a startup (everything after the 8-byte header).
fn startup_params(body: &[u8]) -> Result<Vec<(String, String)>, &'static str> {
    // The list ends with an extra NUL after the last value.
    let body = body
        .strip_suffix(&[0])
        .ok_or("startup: missing final NUL")?;
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let body = body
        .strip_suffix(&[0])
        .ok_or("startup: unterminated value")?;
    let items: Vec<&[u8]> = body.split(|b| *b == 0).collect();
    if !items.len().is_multiple_of(2) {
        return Err("startup: odd key/value count");
    }
    let mut params: Vec<(String, String)> = Vec::new();
    for kv in items.chunks(2) {
        if kv[0].is_empty() {
            return Err("startup: empty key");
        }
        let s = |x: &[u8]| String::from_utf8(x.to_vec()).map_err(|_| "startup: not UTF-8");
        let (k, v) = (s(kv[0])?, s(kv[1])?);
        // Postgres honours the LAST duplicate but `get` would vet the first: refuse the ambiguity.
        if params.iter().any(|(pk, _)| *pk == k) {
            return Err("startup: duplicate key");
        }
        params.push((k, v));
    }
    Ok(params)
}

/// Parse the packet read inside TLS. `packet` is the WHOLE message, length prefix included.
pub fn inside(packet: &[u8]) -> Result<Inside, &'static str> {
    if packet.len() < 8 || packet.len() > MAX_STARTUP {
        return Err("packet: bad size");
    }
    if be32(packet) as usize != packet.len() {
        return Err("packet: length field disagrees with its size");
    }
    let code = be32(&packet[4..8]);
    if code == CANCEL_CODE {
        return if (CANCEL_MIN..=CANCEL_MAX).contains(&packet.len()) {
            Ok(Inside::Cancel(packet.to_vec()))
        } else {
            Err("cancel: bad size")
        };
    }
    // Major version is the high 16 bits; any minor is forwarded as sent (PG18 sends 3.2).
    if code >> 16 != 3 {
        return Err("startup: unsupported protocol major version");
    }
    Ok(Inside::Startup(Startup {
        raw: packet.to_vec(),
        params: startup_params(&packet[8..])?,
    }))
}

/// A Postgres `ErrorResponse` ('E'): severity `FATAL`, SQLSTATE and message, terminated.
pub fn error_response(sqlstate: &str, message: &str) -> Vec<u8> {
    // Fields are NUL-terminated on the wire, so an embedded NUL would corrupt the framing.
    debug_assert!(
        !sqlstate.contains('\0') && !message.contains('\0'),
        "NUL in error field"
    );
    let mut body = Vec::new();
    for (tag, text) in [
        (b'S', "FATAL"),
        (b'V', "FATAL"),
        (b'C', sqlstate),
        (b'M', message),
    ] {
        body.push(tag);
        body.extend_from_slice(text.as_bytes());
        body.push(0);
    }
    body.push(0);
    let mut out = vec![b'E'];
    out.extend_from_slice(&(body.len() as u32 + 4).to_be_bytes());
    out.extend(body);
    out
}

/// A complete backend message at the front of `buf`: its type byte and total length (type byte
/// included), or `None` until it is whole (also for a length field too small to be real).
pub fn frame(buf: &[u8]) -> Option<(u8, usize)> {
    if buf.len() < 5 {
        return None;
    }
    let len = be32(&buf[1..5]) as usize;
    let total = len.checked_add(1)?;
    (len >= 4 && buf.len() >= total).then_some((buf[0], total))
}

/// The process id and secret key of a whole `'K'` (BackendKeyData) message, as the bytes a
/// `CancelRequest` carries. `None` if it is not a well-formed `K` of 4 to 256 key bytes.
pub fn backend_key(msg: &[u8]) -> Option<Vec<u8>> {
    let (kind, total) = frame(msg)?;
    // 5 header bytes + 4 process id + 4..=256 key.
    (kind == b'K' && total == msg.len() && (13..=265).contains(&total)).then(|| msg[5..].to_vec())
}

/// A `CancelRequest` carrying `key` (process id then secret key, as [`backend_key`] returns).
pub fn cancel_request(key: &[u8]) -> Vec<u8> {
    let mut out = ((key.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(&CANCEL_CODE.to_be_bytes());
    out.extend_from_slice(key);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(code: u32) -> [u8; 8] {
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&8u32.to_be_bytes());
        b[4..].copy_from_slice(&code.to_be_bytes());
        b
    }

    /// Build a startup packet by hand: length, protocol, key/value pairs, final NUL.
    fn startup(proto: u32, kvs: &[(&str, &str)]) -> Vec<u8> {
        let mut body = proto.to_be_bytes().to_vec();
        for (k, v) in kvs {
            body.extend_from_slice(k.as_bytes());
            body.push(0);
            body.extend_from_slice(v.as_bytes());
            body.push(0);
        }
        body.push(0);
        let mut out = ((body.len() + 4) as u32).to_be_bytes().to_vec();
        out.extend(body);
        out
    }

    fn k_msg(key_len: usize) -> Vec<u8> {
        let mut m = vec![b'K'];
        m.extend_from_slice(&((8 + key_len) as u32).to_be_bytes());
        m.extend((0..4 + key_len).map(|i| i as u8 ^ 0x5a));
        m
    }

    #[test]
    fn openings() {
        assert_eq!(opening(pair(SSL_CODE)), Opening::Ssl);
        assert_eq!(opening(pair(GSS_CODE)), Opening::Gss);
        assert_eq!(opening(pair(CANCEL_CODE)), Opening::Other);
        assert_eq!(opening([0; 8]), Opening::Other);
    }

    #[test]
    fn startup_round_trips_params() {
        let kvs = [
            ("user", "w"),
            ("database", "d"),
            ("application_name", "a"),
            ("client_encoding", "UTF8"),
        ];
        let p = startup(196_608, &kvs);
        let Ok(Inside::Startup(s)) = inside(&p) else {
            panic!("not a startup")
        };
        assert_eq!(s.raw, p);
        assert_eq!(s.get("user"), Some("w"));
        assert_eq!(s.get("client_encoding"), Some("UTF8"));
        assert_eq!(s.get("nope"), None);
        assert_eq!(s.params.len(), 4);
        // Protocol 3.2 is forwarded as sent.
        assert!(inside(&startup(196_610, &kvs)).is_ok());
    }

    #[test]
    fn bad_startups_are_refused() {
        assert!(inside(&startup(131_072, &[("user", "w")])).is_err()); // protocol 2
        let mut long = startup(196_608, &[("user", "w")]);
        long.resize(MAX_STARTUP + 1, 1);
        let n = long.len() as u32;
        long[..4].copy_from_slice(&n.to_be_bytes());
        assert!(inside(&long).is_err());
        let mut no_nul = startup(196_608, &[("user", "w")]);
        no_nul.pop();
        let n = no_nul.len() as u32;
        no_nul[..4].copy_from_slice(&n.to_be_bytes());
        assert!(inside(&no_nul).is_err());
        // Odd count: a key with no value.
        let mut odd = 196_608u32.to_be_bytes().to_vec();
        odd.extend_from_slice(b"user\0w\0extra\0\0");
        let mut p = ((odd.len() + 4) as u32).to_be_bytes().to_vec();
        p.extend(odd);
        assert!(inside(&p).is_err());
        // Not UTF-8.
        let mut bad = 196_608u32.to_be_bytes().to_vec();
        bad.extend_from_slice(b"user\0\xff\xfe\0\0");
        let mut p = ((bad.len() + 4) as u32).to_be_bytes().to_vec();
        p.extend(bad);
        assert!(inside(&p).is_err());
        // Length field lying about the size.
        let mut lie = startup(196_608, &[("user", "w")]);
        lie[3] += 1;
        assert!(inside(&lie).is_err());
    }

    #[test]
    fn cancel_sizes() {
        for n in [16usize, 44, 268] {
            let p = cancel_request(&[7u8; 260][..n - 8]);
            assert_eq!(p.len(), n);
            assert_eq!(inside(&p), Ok(Inside::Cancel(p.clone())));
        }
        for n in [15usize, 269] {
            let mut p = cancel_request(&[7u8; 260][..8]);
            p.resize(n, 7);
            p[..4].copy_from_slice(&(n as u32).to_be_bytes());
            assert!(inside(&p).is_err());
        }
    }

    #[test]
    fn frame_waits_for_the_whole_message() {
        let m = k_msg(4);
        for cut in 0..m.len() {
            assert_eq!(frame(&m[..cut]), None, "cut {cut}");
        }
        assert_eq!(frame(&m), Some((b'K', m.len())));
        let mut more = m.clone();
        more.push(b'Z');
        assert_eq!(frame(&more), Some((b'K', m.len())));
    }

    #[test]
    fn backend_key_feeds_cancel_request() {
        for key_len in [4usize, 32, 256] {
            let m = k_msg(key_len);
            let key = backend_key(&m).expect("K message");
            assert_eq!(key.len(), 4 + key_len);
            let c = cancel_request(&key);
            assert_eq!(inside(&c), Ok(Inside::Cancel(c.clone())));
            assert_eq!(&c[8..], &m[5..]);
        }
        assert_eq!(backend_key(&k_msg(257)), None);
        let mut z = k_msg(4);
        z[0] = b'Z';
        assert_eq!(backend_key(&z), None);
    }

    #[test]
    fn error_response_bytes() {
        let mut want = vec![b'E', 0, 0, 0, 0];
        want.extend_from_slice(b"SFATAL\0VFATAL\0C28000\0Mx\0\0");
        let n = (want.len() - 1) as u32;
        want[1..5].copy_from_slice(&n.to_be_bytes());
        assert_eq!(error_response("28000", "x"), want);
    }

    /// Wrap a raw body (after the length) into a packet with a correct length field.
    fn packet(body: &[u8]) -> Vec<u8> {
        let mut p = ((body.len() + 4) as u32).to_be_bytes().to_vec();
        p.extend_from_slice(body);
        p
    }

    #[test]
    fn duplicate_keys_are_refused() {
        // Postgres honours the LAST duplicate; the gateway vets the first. Refuse both ways.
        for key in ["user", "database"] {
            let p = startup(196_608, &[(key, "a"), (key, "b")]);
            assert!(inside(&p).is_err(), "duplicate {key}");
        }
    }

    #[test]
    fn max_startup_boundary() {
        let sized = |n: usize| {
            // Fixed overhead: 8 header + "user\0w\0" 7 + "application_name\0" 17 + value NUL + final NUL.
            let pad = n - (8 + 7 + 17 + 1 + 1);
            let app = "x".repeat(pad);
            startup(196_608, &[("user", "w"), ("application_name", &app)])
        };
        assert_eq!(sized(MAX_STARTUP).len(), MAX_STARTUP);
        assert!(inside(&sized(MAX_STARTUP)).is_ok());
        assert_eq!(sized(MAX_STARTUP + 1).len(), MAX_STARTUP + 1);
        assert!(inside(&sized(MAX_STARTUP + 1)).is_err());
    }

    #[test]
    fn short_inputs_are_refused_not_panicked() {
        for n in [0usize, 3, 4, 7] {
            assert!(inside(&vec![0u8; n]).is_err(), "{n} bytes");
        }
    }

    #[test]
    fn more_refusals() {
        // Right code, wrong length: not an SSLRequest.
        let mut p = pair(SSL_CODE);
        p[..4].copy_from_slice(&9u32.to_be_bytes());
        assert_eq!(opening(p), Opening::Other);
        let mut p = pair(GSS_CODE);
        p[..4].copy_from_slice(&9u32.to_be_bytes());
        assert_eq!(opening(p), Opening::Other);
        // Protocol major 4.
        assert!(inside(&startup(262_144, &[("user", "w")])).is_err());
        // Cancel whose length field disagrees with its size.
        let mut c = cancel_request(&[7u8; 8]);
        c[3] += 1;
        assert!(inside(&c).is_err());
        // Empty key.
        let mut b = 196_608u32.to_be_bytes().to_vec();
        b.extend_from_slice(b"\0v\0\0");
        assert!(inside(&packet(&b)).is_err());
        // Empty parameter list is fine.
        let mut b = 196_608u32.to_be_bytes().to_vec();
        b.push(0);
        let Ok(Inside::Startup(s)) = inside(&packet(&b)) else {
            panic!("not a startup")
        };
        assert!(s.params.is_empty());
        // backend_key: key below the floor, and trailing bytes after the message.
        assert_eq!(backend_key(&k_msg_sized(3)), None);
        let mut m = k_msg(4);
        m.push(0);
        assert_eq!(backend_key(&m), None);
        // frame: a length field below 4.
        assert_eq!(frame(&[b'K', 0, 0, 0, 3, 0, 0, 0]), None);
    }

    fn k_msg_sized(key_len: usize) -> Vec<u8> {
        let mut m = vec![b'K'];
        m.extend_from_slice(&((8 + key_len) as u32).to_be_bytes());
        m.extend(std::iter::repeat_n(1u8, 4 + key_len));
        m
    }

    #[test]
    #[should_panic(expected = "NUL")]
    fn error_response_refuses_nul_in_arguments() {
        error_response("28000", "a\0b");
    }
}
