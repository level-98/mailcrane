//! Every seed in `tests/fixtures/sieve/` parses as a stream of responses.

use mailcrane_sieve::{parse_response, Status};
use std::path::Path;

#[test]
fn fixtures_parse_completely() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures/sieve");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("fixture folder") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("sieve") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("read");
        let mut rest = bytes.as_slice();
        let mut statuses = Vec::new();
        while !rest.is_empty() {
            let (r, used) = parse_response(rest).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            statuses.push(r.status);
            rest = &rest[used..];
        }
        assert!(!statuses.is_empty());
        if path.ends_with("getscript-quota.sieve") {
            assert_eq!(statuses, [Status::Ok, Status::No, Status::Bye]);
        }
        seen += 1;
    }
    assert_eq!(seen, 3);
}
