//! Reading SRV (RFC 2782) and MX (RFC 1035 §3.3.9) answers out of a raw DNS
//! response message (RFC 1035 §4).
//!
//! The query itself is made by the platform's own resolver, so lookups go
//! to the user's configured DNS server and nowhere else; the core only
//! parses the bytes that come back. Every read is bounds-checked and name
//! compression can't loop.

/// The record types discovery asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordType {
    Srv,
    Mx,
}

impl RecordType {
    /// The type's number on the wire.
    pub fn code(self) -> u16 {
        match self {
            RecordType::Srv => 33,
            RecordType::Mx => 15,
        }
    }
}

/// One SRV record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Srv {
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    /// Lowercase, without the trailing dot. `"."` (empty) means "no
    /// service here" (RFC 2782).
    pub target: String,
}

/// One MX record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mx {
    pub preference: u16,
    pub exchange: String,
}

/// Most answer records read from one message.
const MAX_ANSWERS: usize = 64;

struct Reader<'a> {
    msg: &'a [u8],
}

impl Reader<'_> {
    fn u16_at(&self, at: usize) -> Option<u16> {
        let b = self.msg.get(at..at.checked_add(2)?)?;
        Some(u16::from_be_bytes([*b.first()?, *b.get(1)?]))
    }

    /// The name starting at `at`, and where the bytes after it start.
    fn name_at(&self, mut at: usize) -> Option<(String, usize)> {
        let mut labels: Vec<String> = Vec::new();
        let mut end = None;
        let mut jumps = 0;
        let mut total = 0usize;
        loop {
            let len = *self.msg.get(at)?;
            match len & 0xC0 {
                0x00 if len == 0 => {
                    let next = end.unwrap_or(at + 1);
                    return Some((labels.join(".").to_ascii_lowercase(), next));
                }
                0x00 => {
                    let n = usize::from(len);
                    let label = self.msg.get(at + 1..at + 1 + n)?;
                    total += n + 1;
                    if total > 255 {
                        return None;
                    }
                    labels.push(String::from_utf8_lossy(label).into_owned());
                    at += 1 + n;
                }
                0xC0 => {
                    // A pointer (RFC 1035 §4.1.4). Bounded jumps stop loops.
                    jumps += 1;
                    if jumps > 32 {
                        return None;
                    }
                    let target = usize::from(self.u16_at(at)? & 0x3FFF);
                    if end.is_none() {
                        end = Some(at + 2);
                    }
                    at = target;
                }
                _ => return None,
            }
        }
    }
}

/// The answer records of type `rtype` as `(rdata offset, rdata length)`.
/// A response that isn't a successful answer gives no records.
fn answers(msg: &[u8], rtype: RecordType) -> Option<Vec<(usize, usize)>> {
    let r = Reader { msg };
    let flags = r.u16_at(2)?;
    let is_response = flags & 0x8000 != 0;
    let rcode = flags & 0x000F;
    if !is_response || rcode != 0 {
        return Some(Vec::new());
    }
    let qd = usize::from(r.u16_at(4)?);
    let an = usize::from(r.u16_at(6)?).min(MAX_ANSWERS);
    let mut at = 12;
    for _ in 0..qd.min(8) {
        let (_, next) = r.name_at(at)?;
        at = next + 4;
    }
    let mut out = Vec::new();
    for _ in 0..an {
        let (_, next) = r.name_at(at)?;
        let kind = r.u16_at(next)?;
        let class = r.u16_at(next + 2)?;
        let len = usize::from(r.u16_at(next + 8)?);
        let data = next + 10;
        msg.get(data..data + len)?;
        if kind == rtype.code() && class == 1 {
            out.push((data, len));
        }
        at = data + len;
    }
    Some(out)
}

/// SRV records in a response, sorted by priority then by weight (highest
/// first). Malformed messages give `None`.
pub fn parse_srv(msg: &[u8]) -> Option<Vec<Srv>> {
    let r = Reader { msg };
    let mut out = Vec::new();
    for (at, len) in answers(msg, RecordType::Srv)? {
        if len < 7 {
            return None;
        }
        let (target, _) = r.name_at(at + 6)?;
        out.push(Srv {
            priority: r.u16_at(at)?,
            weight: r.u16_at(at + 2)?,
            port: r.u16_at(at + 4)?,
            target,
        });
    }
    out.sort_by(|a, b| a.priority.cmp(&b.priority).then(b.weight.cmp(&a.weight)));
    Some(out)
}

/// MX records in a response, most preferred first.
pub fn parse_mx(msg: &[u8]) -> Option<Vec<Mx>> {
    let r = Reader { msg };
    let mut out = Vec::new();
    for (at, len) in answers(msg, RecordType::Mx)? {
        if len < 3 {
            return None;
        }
        let (exchange, _) = r.name_at(at + 2)?;
        out.push(Mx {
            preference: r.u16_at(at)?,
            exchange,
        });
    }
    out.sort_by_key(|m| m.preference);
    Some(out)
}

/// Builds DNS response messages for tests (also used by the discovery
/// tests' fake resolver).
#[cfg(test)]
pub(crate) mod build {
    use super::RecordType;

    fn name(out: &mut Vec<u8>, n: &str) {
        for label in n.split('.').filter(|l| !l.is_empty()) {
            out.push(u8::try_from(label.len()).expect("short label"));
            out.extend_from_slice(label.as_bytes());
        }
        out.push(0);
    }

    /// A response to `qname` with these rdata payloads (already encoded).
    pub fn response(qname: &str, rtype: RecordType, rcode: u8, rdatas: &[Vec<u8>]) -> Vec<u8> {
        let mut m = vec![0x12, 0x34, 0x81, 0x80 | rcode, 0, 1];
        m.extend_from_slice(&u16::try_from(rdatas.len()).expect("few").to_be_bytes());
        m.extend_from_slice(&[0, 0, 0, 0]);
        name(&mut m, qname);
        m.extend_from_slice(&rtype.code().to_be_bytes());
        m.extend_from_slice(&[0, 1]);
        for rd in rdatas {
            // Owner name as a pointer to the question (offset 12).
            m.extend_from_slice(&[0xC0, 12]);
            m.extend_from_slice(&rtype.code().to_be_bytes());
            m.extend_from_slice(&[0, 1, 0, 0, 0x0E, 0x10]);
            m.extend_from_slice(&u16::try_from(rd.len()).expect("small").to_be_bytes());
            m.extend_from_slice(rd);
        }
        m
    }

    pub fn srv(priority: u16, weight: u16, port: u16, target: &str) -> Vec<u8> {
        let mut rd = Vec::new();
        rd.extend_from_slice(&priority.to_be_bytes());
        rd.extend_from_slice(&weight.to_be_bytes());
        rd.extend_from_slice(&port.to_be_bytes());
        name(&mut rd, target);
        rd
    }

    pub fn mx(preference: u16, exchange: &str) -> Vec<u8> {
        let mut rd = preference.to_be_bytes().to_vec();
        name(&mut rd, exchange);
        rd
    }
}

#[cfg(test)]
mod tests {
    use super::build::*;
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn srv_answers_sort_by_priority_and_weight() {
        let msg = response(
            "_imaps._tcp.example.org",
            RecordType::Srv,
            0,
            &[
                srv(20, 0, 993, "backup.example.org"),
                srv(10, 5, 993, "Imap2.Example.org"),
                srv(10, 50, 993, "imap.example.org."),
            ],
        );
        let list = parse_srv(&msg).expect("parses");
        let targets: Vec<&str> = list.iter().map(|s| s.target.as_str()).collect();
        assert_eq!(
            targets,
            [
                "imap.example.org",
                "imap2.example.org",
                "backup.example.org"
            ]
        );
    }

    #[test]
    fn mx_answers_and_errors() {
        let msg = response(
            "example.org",
            RecordType::Mx,
            0,
            &[mx(20, "alt.mx.example.net"), mx(5, "mx.example.net")],
        );
        let list = parse_mx(&msg).expect("parses");
        assert_eq!(list[0].exchange, "mx.example.net");
        // NXDOMAIN (rcode 3) is no records, not an error.
        let nx = response("nope.example", RecordType::Mx, 3, &[]);
        assert_eq!(parse_mx(&nx), Some(Vec::new()));
        // An answer of another type is skipped.
        let other = response(
            "example.org",
            RecordType::Srv,
            0,
            &[srv(1, 1, 993, "x.org")],
        );
        assert_eq!(parse_mx(&other), Some(Vec::new()));
    }

    #[test]
    fn compression_loops_and_truncation_are_refused() {
        // A name that points at itself.
        let mut looped = response("example.org", RecordType::Mx, 0, &[vec![0, 5, 0xC0, 0]]);
        let len = looped.len();
        // Point the exchange at its own offset.
        let self_at = u8::try_from(len - 2).expect("short");
        looped[len - 1] = self_at;
        looped[len - 2] = 0xC0;
        assert_eq!(parse_mx(&looped), None);
        let good = response("example.org", RecordType::Mx, 0, &[mx(5, "mx.example.net")]);
        for cut in 0..good.len() {
            let _ = parse_mx(&good[..cut]);
        }
        assert_eq!(parse_mx(&good[..good.len() - 3]), None);
    }

    proptest! {
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..600)) {
            let _ = parse_srv(&bytes);
            let _ = parse_mx(&bytes);
        }
    }
}
