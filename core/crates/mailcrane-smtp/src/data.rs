//! Preparing message bytes for the DATA command (RFC 5321 §4.1.1.4, §4.5.2).

/// Normalises line endings to CRLF, removes `Bcc:` headers, dot-stuffs, and
/// adds the end-of-data marker.
///
/// A lone CR or LF is turned into CRLF so a server that treats it as a line
/// end cannot see a premature `.` terminator inside the body.
pub(crate) fn prepare(message: &[u8]) -> Vec<u8> {
    let mut lines: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while let Some(&b) = message.get(i) {
        let end = match b {
            b'\r' if message.get(i + 1) == Some(&b'\n') => Some(2),
            b'\r' | b'\n' => Some(1),
            _ => None,
        };
        match end {
            Some(len) => {
                lines.push(message.get(start..i).unwrap_or_default());
                i += len;
                start = i;
            }
            None => i += 1,
        }
    }
    let tail = message.get(start..).unwrap_or_default();
    if !tail.is_empty() {
        lines.push(tail);
    }

    let mut out = Vec::with_capacity(message.len() + 16);
    let mut in_headers = true;
    let mut skipping_bcc = false;
    for line in lines {
        if in_headers {
            if line.is_empty() {
                in_headers = false;
                skipping_bcc = false;
            } else if matches!(line.first(), Some(b' ' | b'\t')) {
                // RFC 5322 §2.2.3: a folded continuation of the previous header.
                if skipping_bcc {
                    continue;
                }
            } else {
                skipping_bcc = is_bcc(line);
                if skipping_bcc {
                    continue;
                }
            }
        }
        if line.first() == Some(&b'.') {
            out.push(b'.');
        }
        out.extend_from_slice(line);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

fn is_bcc(line: &[u8]) -> bool {
    let name_end = line.iter().position(|&b| b == b':');
    match name_end.and_then(|end| line.get(..end)) {
        Some(name) => name.trim_ascii_end().eq_ignore_ascii_case(b"bcc"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::prepare;

    #[test]
    fn strips_bcc_with_folding_only_in_headers() {
        let m = b"To: a@example.org\r\nBcc: b@example.org,\r\n c@example.org\r\nSubject: x\r\n\r\nBcc: in body\r\n";
        let out = prepare(m);
        assert_eq!(
            out,
            b"To: a@example.org\r\nSubject: x\r\n\r\nBcc: in body\r\n.\r\n"
        );
    }

    #[test]
    fn stuffs_dots_and_normalises_endings() {
        let out = prepare(b"Subject: x\n\n.hidden\r.\nend");
        assert_eq!(out, b"Subject: x\r\n\r\n..hidden\r\n..\r\nend\r\n.\r\n");
    }

    #[test]
    fn bcc_with_space_before_colon_is_stripped() {
        assert_eq!(
            prepare(b"Bcc : x@example.org\r\nA: b\r\n\r\n"),
            b"A: b\r\n\r\n.\r\n"
        );
    }
}
